//! Mission manager — orchestrates long-running goals that spawn threads over time.
//!
//! Missions track ongoing objectives and periodically spawn threads to make
//! progress. The manager handles lifecycle (create, pause, resume, complete)
//! and delegates thread spawning to [`ThreadManager`].

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use ironclaw_skills::types::ActivationCriteria;
use ironclaw_skills::v2::{CodeSnippet, SkillRepairRecord, SkillRepairType, V2SkillMetadata};

use crate::executor::trace::{ExecutionTrace, IssueSeverity};
use crate::memory::{RetrievalEngine, SkillTracker};
use crate::runtime::manager::ThreadManager;
use crate::runtime::messaging::ThreadOutcome;
use crate::traits::store::Store;
use crate::traits::workspace::WorkspaceReader;
use crate::types::error::EngineError;
use crate::types::memory::{DocId, DocType, MemoryDoc};
use crate::types::mission::{
    Mission, MissionCadence, MissionId, MissionStatus, next_cron_fire, next_cron_fire_required,
};
use crate::types::project::ProjectId;
use crate::types::shared_owner_id;
use crate::types::thread::{
    ActiveSkillProvenance, Thread, ThreadConfig, ThreadId, ThreadState, ThreadType,
};

/// Per-mission compiled regex cache. We compile patterns lazily on first
/// match attempt and discard them when the mission updates or deletes its
/// cadence. The cache is process-local — restarts repopulate on demand.
type EventRegexCache = HashMap<MissionId, regex::Regex>;

/// Maximum compiled regex size, mirroring the v1 routine engine. Patterns
/// that exceed this are refused at compile time so a hostile or buggy
/// mission cannot pin the matcher with a pathological regex.
const MAX_EVENT_REGEX_SIZE: usize = 64 * 1024;

/// Per-user fire-rate ceiling expressed as a token bucket. Independent of
/// per-mission `cooldown_secs`, this is a *global* cap across all of a
/// user's missions so a user that owns many event-triggered missions can't
/// collectively flood the LLM.
#[derive(Debug, Clone)]
pub struct FireRateLimit {
    /// Maximum number of fires permitted within `window`.
    pub max_fires: u32,
    /// Sliding-window duration. Fires older than this are evicted.
    pub window: std::time::Duration,
}

impl Default for FireRateLimit {
    /// 100 mission firings per user per hour. Generous enough that normal
    /// cron + a handful of event-driven missions don't notice it; tight
    /// enough that a misbehaving pattern is bounded.
    fn default() -> Self {
        Self {
            max_fires: 100,
            window: std::time::Duration::from_secs(3600),
        }
    }
}

/// Engine-side budget abstraction. Implementations decide whether the
/// `user_id` still has enough LLM/financial budget to spawn another
/// mission thread. The host implements this over its existing
/// `CostGuard`.
///
/// When `MissionManager` has no `BudgetGate` attached, all fires are
/// allowed (back-compat for embedders that don't use a budget).
#[async_trait::async_trait]
pub trait BudgetGate: Send + Sync {
    /// Returns `true` if a mission fire is allowed for `user_id`. The
    /// `mission_id` is included so adapters can apply per-mission policies
    /// if they wish; most implementations will only consult `user_id`.
    async fn allow_mission_fire(&self, user_id: &str, mission_id: MissionId) -> bool;
}

/// Notification emitted when a mission thread completes.
///
/// The bridge subscribes to these and routes the response text to
/// the mission's `notify_channels` via `ChannelManager::broadcast()`.
#[derive(Debug, Clone)]
pub struct MissionNotification {
    pub mission_id: MissionId,
    pub mission_name: String,
    pub thread_id: ThreadId,
    pub user_id: String,
    /// Channels to notify (from `Mission.notify_channels`).
    pub notify_channels: Vec<String>,
    /// Optional per-channel recipient (from `Mission.notify_user`). When
    /// `None`, the channel's default recipient is used.
    pub notify_user: Option<String>,
    /// The thread's response text (None if failed/no output).
    pub response: Option<String>,
    /// True if the thread failed.
    pub is_error: bool,
}

/// Optional updates to apply to a mission via [`MissionManager::update_mission`].
#[derive(Debug, Default, Clone)]
pub struct MissionUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub goal: Option<String>,
    pub cadence: Option<MissionCadence>,
    pub notify_channels: Option<Vec<String>>,
    pub notify_user: Option<String>,
    pub context_paths: Option<Vec<String>>,
    pub max_threads_per_day: Option<u32>,
    pub success_criteria: Option<String>,
    pub cooldown_secs: Option<u64>,
    pub max_concurrent: Option<u32>,
    pub dedup_window_secs: Option<u64>,
}

/// In-memory dedup state for event-triggered missions. Keyed by
/// (mission_id, dedup-key) → last fire timestamp.
type DedupKey = (MissionId, String);

/// Manages mission lifecycle and thread spawning.
pub struct MissionManager {
    store: Arc<dyn Store>,
    thread_manager: Arc<ThreadManager>,
    /// Active missions indexed by ID for quick lookup.
    active: RwLock<Vec<MissionId>>,
    /// Broadcast channel for mission outcome notifications.
    notification_tx: tokio::sync::broadcast::Sender<MissionNotification>,
    /// Per-mission in-memory cooldown timestamp, recorded after each
    /// `fire_mission` attempt regardless of whether `save_mission` succeeded.
    ///
    /// `tick` consults this to suppress re-firing the same mission within
    /// [`FIRE_COOLDOWN`] when a transient store failure has prevented
    /// `next_fire_at` / `threads_today` from advancing in the persisted record.
    /// Without this guard, a save failure after a successful fire would cause
    /// the next 60 s tick (and every subsequent tick) to re-fire the same
    /// mission until the store recovers, spawning duplicate threads up to the
    /// daily budget.
    last_fire_attempt: RwLock<HashMap<MissionId, chrono::DateTime<chrono::Utc>>>,
    /// Optional workspace reader used to load `Mission.context_paths` at
    /// fire time. When `None`, context preloading is silently skipped.
    workspace: Option<Arc<dyn WorkspaceReader>>,
    /// Per-mission dedup table for event-triggered firings. Cleared
    /// opportunistically when entries fall outside the dedup window.
    dedup_table: RwLock<HashMap<DedupKey, chrono::DateTime<chrono::Utc>>>,
    /// Compiled regex cache for `OnEvent` mission patterns. Lazily filled
    /// on first match attempt; entries are evicted on mission update/delete.
    event_regex_cache: RwLock<EventRegexCache>,
    /// Per-user sliding-window fire log used by the global rate limiter.
    /// Each `VecDeque` holds firing timestamps within the configured window.
    user_fire_log: RwLock<HashMap<String, VecDeque<chrono::DateTime<chrono::Utc>>>>,
    /// Global per-user fire-rate ceiling.
    rate_limit: FireRateLimit,
    /// Optional budget gate consulted before each fire.
    budget_gate: Option<Arc<dyn BudgetGate>>,
}

/// Minimum gap between successive `fire_mission` attempts for the same
/// mission ID, enforced in-memory by `tick`. Chosen to comfortably exceed the
/// 60 s tick interval so a single tick gap is always honored, while still
/// allowing recovery within a few minutes if the store comes back.
const FIRE_COOLDOWN: Duration = Duration::from_secs(90);

impl MissionManager {
    pub fn new(store: Arc<dyn Store>, thread_manager: Arc<ThreadManager>) -> Self {
        let (notification_tx, _) = tokio::sync::broadcast::channel(64);
        Self {
            store,
            thread_manager,
            active: RwLock::new(Vec::new()),
            notification_tx,
            last_fire_attempt: RwLock::new(HashMap::new()),
            workspace: None,
            dedup_table: RwLock::new(HashMap::new()),
            event_regex_cache: RwLock::new(HashMap::new()),
            user_fire_log: RwLock::new(HashMap::new()),
            rate_limit: FireRateLimit::default(),
            budget_gate: None,
        }
    }

    /// Access the underlying store (for ownership validation on cross-project
    /// operations like mission creation with explicit project_id).
    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// Attach a workspace reader so `context_paths` are loaded at fire time.
    /// Builder-style for back-compat with existing call sites that don't yet
    /// supply a reader.
    pub fn with_workspace_reader(mut self, reader: Arc<dyn WorkspaceReader>) -> Self {
        self.workspace = Some(reader);
        self
    }

    /// Attach a budget gate so each fire consults the host's spend limit.
    /// When unattached, all fires are allowed (back-compat).
    pub fn with_budget_gate(mut self, gate: Arc<dyn BudgetGate>) -> Self {
        self.budget_gate = Some(gate);
        self
    }

    /// Override the per-user fire-rate limit. Defaults to 100 fires/hour.
    pub fn with_rate_limit(mut self, limit: FireRateLimit) -> Self {
        self.rate_limit = limit;
        self
    }

    /// Subscribe to mission outcome notifications.
    ///
    /// The bridge uses this to route mission results to channels.
    pub fn subscribe_notifications(&self) -> tokio::sync::broadcast::Receiver<MissionNotification> {
        self.notification_tx.subscribe()
    }

    /// Test-only handle to the broadcast sender so unit tests can drive
    /// `process_mission_outcome_and_notify` without going through the full
    /// thread lifecycle. Not part of the public API.
    #[cfg(test)]
    pub(crate) fn notification_tx_for_test(
        &self,
    ) -> &tokio::sync::broadcast::Sender<MissionNotification> {
        &self.notification_tx
    }

    /// Populate the active mission index from persisted mission state.
    ///
    /// Also backfills `next_fire_at` for active cron missions created before
    /// the scheduling fix — without this, legacy cron missions would remain
    /// stuck with `next_fire_at = None` and never fire.
    pub async fn bootstrap_project(&self, project_id: ProjectId) -> Result<usize, EngineError> {
        // System operation: load all missions for the project regardless of user.
        let missions = self.store.list_all_missions(project_id).await?;
        let mut active_ids = Vec::new();

        for mission in missions {
            if mission.status != MissionStatus::Active {
                continue;
            }
            // Backfill next_fire_at for cron missions that predate the
            // scheduling fix. Match all three branches of next_cron_fire so a
            // mission with an unschedulable cron (Ok(None) — e.g. a year-locked
            // expression in the past) or an invalid expression (Err) is at
            // least observable in the logs instead of silently staying stuck.
            //
            // Lenient `next_cron_fire` (not `_required`): startup backfill must
            // never block — a single corrupt persisted expression cannot fail
            // bootstrap, since the rest of the active missions still need to
            // register. See `next_cron_fire_required` for the strict variant
            // used at lifecycle entry points.
            if let MissionCadence::Cron {
                ref expression,
                ref timezone,
            } = mission.cadence
                && mission.next_fire_at.is_none()
            {
                match next_cron_fire(expression, timezone.as_ref()) {
                    Ok(Some(next)) => {
                        // Re-load the mission immediately before save to narrow
                        // the TOCTOU window between the initial list_all_missions
                        // snapshot and our save. If a concurrent fire/update has
                        // already populated next_fire_at, skip — that writer's
                        // copy is fresher than ours. The remaining race window
                        // (between this re-load and save_mission) is much smaller
                        // than the original list-then-save window, and a strict
                        // CAS would require a new Store trait method.
                        match self.store.load_mission(mission.id).await {
                            Ok(Some(mut fresh)) if fresh.next_fire_at.is_none() => {
                                fresh.next_fire_at = Some(next);
                                match self.store.save_mission(&fresh).await {
                                    Ok(()) => debug!(
                                        mission_id = %mission.id,
                                        next = %next,
                                        "backfilled next_fire_at for legacy cron mission"
                                    ),
                                    Err(e) => debug!(
                                        mission_id = %mission.id,
                                        error = %e,
                                        "failed to persist next_fire_at backfill; mission will retry on next bootstrap"
                                    ),
                                }
                            }
                            Ok(Some(_)) => debug!(
                                mission_id = %mission.id,
                                "next_fire_at already set by concurrent writer; skipping backfill"
                            ),
                            Ok(None) => debug!(
                                mission_id = %mission.id,
                                "mission deleted between bootstrap list and backfill; skipping"
                            ),
                            Err(e) => debug!(
                                mission_id = %mission.id,
                                error = %e,
                                "failed to re-load mission for backfill"
                            ),
                        }
                    }
                    Ok(None) => debug!(
                        mission_id = %mission.id,
                        expression = %expression,
                        timezone = ?timezone,
                        "legacy cron mission has no upcoming fire time; leaving next_fire_at unset"
                    ),
                    Err(e) => debug!(
                        mission_id = %mission.id,
                        expression = %expression,
                        timezone = ?timezone,
                        error = %e,
                        "failed to compute next_fire_at for legacy cron mission; leaving next_fire_at unset"
                    ),
                }
            }
            active_ids.push(mission.id);
        }

        let count = active_ids.len();
        *self.active.write().await = active_ids;
        debug!(project_id = ?project_id, active_missions = count, "bootstrapped active missions");
        Ok(count)
    }

    /// Create and persist a new mission. Returns the mission ID.
    pub async fn create_mission(
        &self,
        project_id: ProjectId,
        user_id: impl Into<String>,
        name: impl Into<String>,
        goal: impl Into<String>,
        cadence: MissionCadence,
        notify_channels: Vec<String>,
    ) -> Result<MissionId, EngineError> {
        let mut mission = Mission::new(project_id, user_id, name, goal, cadence);
        if let MissionCadence::Cron {
            ref expression,
            ref timezone,
        } = mission.cadence
        {
            // Reject Ok(None) at the create boundary — an Active cron mission
            // with `next_fire_at = None` is the original #1944 failure mode.
            mission.next_fire_at = Some(next_cron_fire_required(expression, timezone.as_ref())?);
        }
        mission.notify_channels = notify_channels;
        let id = mission.id;
        self.store.save_mission(&mission).await?;
        self.active.write().await.push(id);
        debug!(mission_id = %id, "mission created");
        Ok(id)
    }

    /// Update mutable fields on a mission. Only non-None fields are applied.
    pub async fn update_mission(
        &self,
        id: MissionId,
        user_id: &str,
        updates: MissionUpdate,
    ) -> Result<(), EngineError> {
        let mut mission = self
            .store
            .load_mission(id)
            .await?
            .ok_or_else(|| EngineError::Store {
                reason: format!("mission {id} not found"),
            })?;

        let allowed = if mission.owner_id().is_shared() {
            crate::types::is_shared_owner(user_id)
        } else {
            mission.is_owned_by(user_id)
        };
        if !allowed {
            return Err(EngineError::AccessDenied {
                user_id: user_id.to_string(),
                entity: format!("mission {id}"),
            });
        }

        if let Some(name) = updates.name {
            mission.name = name;
        }
        if let Some(description) = updates.description {
            mission.description = Some(description);
        }
        if let Some(goal) = updates.goal {
            mission.goal = goal;
        }
        if let Some(cadence) = updates.cadence {
            mission.cadence = cadence;
            // Recompute scheduling state to match the new cadence. Without this,
            // a Manual -> Cron switch leaves next_fire_at = None and the ticker
            // never picks the mission up; a Cron expression/timezone change
            // keeps firing on the old schedule until the mission is paused and
            // resumed. Clear next_fire_at for non-cron cadences so a stale
            // value can't trigger an unrelated cron path. Reject cron schedules
            // that are valid but have no future fire time so we don't persist
            // an Active mission that can never run.
            //
            // Strict `next_cron_fire_required`: an `Err` here returns from
            // `update_mission` BEFORE the `save_mission` call below, leaving
            // the persisted record on its previous (valid) cadence. The
            // `mission` local is dropped without ever being persisted —
            // `save_mission` is the only persistence boundary in this
            // function, so failing before it leaves the store untouched.
            // Verified by `update_mission_rejects_switch_to_unschedulable_cron`.
            mission.next_fire_at = match &mission.cadence {
                MissionCadence::Cron {
                    expression,
                    timezone,
                } => Some(next_cron_fire_required(expression, timezone.as_ref())?),
                _ => None,
            };
        }
        if let Some(channels) = updates.notify_channels {
            mission.notify_channels = channels;
        }
        if let Some(notify_user) = updates.notify_user {
            mission.notify_user = Some(notify_user);
        }
        if let Some(context_paths) = updates.context_paths {
            mission.context_paths = context_paths;
        }
        if let Some(max) = updates.max_threads_per_day {
            mission.max_threads_per_day = max;
        }
        if let Some(criteria) = updates.success_criteria {
            mission.success_criteria = Some(criteria);
        }
        if let Some(secs) = updates.cooldown_secs {
            mission.cooldown_secs = secs;
        }
        if let Some(max) = updates.max_concurrent {
            mission.max_concurrent = max;
        }
        if let Some(secs) = updates.dedup_window_secs {
            mission.dedup_window_secs = secs;
        }

        mission.updated_at = chrono::Utc::now();
        self.store.save_mission(&mission).await?;
        // The cadence (and therefore event_pattern) may have changed.
        // Drop the cached compiled regex; the next match attempt
        // recompiles from the current pattern.
        self.evict_event_regex(id).await;
        debug!(mission_id = %id, "mission updated");
        Ok(())
    }

    /// Pause an active mission. No new threads will be spawned.
    ///
    /// Shared missions can only be managed by shared owners (system user).
    pub async fn pause_mission(&self, id: MissionId, user_id: &str) -> Result<(), EngineError> {
        let mission = self
            .store
            .load_mission(id)
            .await?
            .ok_or_else(|| EngineError::Store {
                reason: format!("mission {id} not found"),
            })?;
        let allowed = if mission.owner_id().is_shared() {
            crate::types::is_shared_owner(user_id)
        } else {
            mission.is_owned_by(user_id)
        };
        if !allowed {
            return Err(EngineError::AccessDenied {
                user_id: user_id.to_string(),
                entity: format!("mission {id}"),
            });
        }
        self.store
            .update_mission_status(id, MissionStatus::Paused)
            .await?;
        self.active.write().await.retain(|mid| *mid != id);
        // Drop the in-memory cooldown entry — a paused mission can't fire,
        // so the cooldown is dead state and would otherwise leak until the
        // process restarts.
        self.last_fire_attempt.write().await.remove(&id);
        debug!(mission_id = %id, "mission paused");
        Ok(())
    }

    /// Resume a paused mission.
    ///
    /// Shared missions can only be managed by shared owners (system user).
    /// Only `Paused` missions can be resumed — `Completed` and `Failed` are
    /// terminal states and must not be resurrected by a stray resume call,
    /// so anything else is rejected with a `Store` error.
    pub async fn resume_mission(&self, id: MissionId, user_id: &str) -> Result<(), EngineError> {
        let mut mission = self
            .store
            .load_mission(id)
            .await?
            .ok_or_else(|| EngineError::Store {
                reason: format!("mission {id} not found"),
            })?;
        let allowed = if mission.owner_id().is_shared() {
            crate::types::is_shared_owner(user_id)
        } else {
            mission.is_owned_by(user_id)
        };
        if !allowed {
            return Err(EngineError::AccessDenied {
                user_id: user_id.to_string(),
                entity: format!("mission {id}"),
            });
        }
        if mission.status != MissionStatus::Paused {
            return Err(EngineError::Store {
                reason: format!(
                    "mission {id} is in state {:?}, only Paused missions can be resumed",
                    mission.status
                ),
            });
        }
        // Mutate-and-save in a single round-trip. The previous implementation
        // did `update_mission_status(Active)` and then a separate `load+save`
        // to recompute next_fire_at — between the two writes, a concurrent
        // `update_mission`/`fire_mission` could modify other fields that the
        // second save would then silently overwrite with the stale reload.
        mission.status = MissionStatus::Active;
        if let MissionCadence::Cron {
            ref expression,
            ref timezone,
        } = mission.cadence
        {
            // Reject Ok(None): resuming a cron mission whose schedule has no
            // upcoming fire time would silently re-create the #1944 stuck
            // state. Surface the error so the caller can fix the schedule.
            mission.next_fire_at = Some(next_cron_fire_required(expression, timezone.as_ref())?);
        }
        mission.updated_at = chrono::Utc::now();
        self.store.save_mission(&mission).await?;
        let mut active = self.active.write().await;
        if !active.contains(&id) {
            active.push(id);
        }
        debug!(mission_id = %id, "mission resumed");
        Ok(())
    }

    /// Mark a mission as completed.
    pub async fn complete_mission(&self, id: MissionId) -> Result<(), EngineError> {
        self.store
            .update_mission_status(id, MissionStatus::Completed)
            .await?;
        self.active.write().await.retain(|mid| *mid != id);
        // Terminal state — drop the cooldown entry so the in-memory map
        // doesn't accumulate an entry per mission ever fired.
        self.last_fire_attempt.write().await.remove(&id);
        self.evict_event_regex(id).await;
        debug!(mission_id = %id, "mission completed");
        Ok(())
    }

    /// Fire a mission — build meta-prompt, spawn thread, process outcome.
    ///
    /// Optional `trigger_payload` carries webhook/event data that triggered this
    /// fire. It's injected into the thread's context as `state["trigger_payload"]`.
    pub async fn fire_mission(
        &self,
        id: MissionId,
        user_id: &str,
        trigger_payload: Option<serde_json::Value>,
    ) -> Result<Option<ThreadId>, EngineError> {
        let mission = self.store.load_mission(id).await?;
        let mission = match mission {
            Some(m) => m,
            None => {
                return Err(EngineError::Store {
                    reason: format!("mission {id} not found"),
                });
            }
        };

        // Tenant isolation: verify the requesting user owns this mission.
        // Shared missions can be fired by any user — the spawned
        // thread inherits the requesting user's identity, keeping artifacts user-scoped.
        if !mission.owner_id().is_shared() && !mission.is_owned_by(user_id) {
            return Err(EngineError::AccessDenied {
                user_id: user_id.to_string(),
                entity: format!("mission {id}"),
            });
        }

        if mission.is_terminal() {
            debug!(mission_id = %id, status = ?mission.status, "cannot fire terminal mission");
            return Ok(None);
        }

        // Check daily budget
        if mission.max_threads_per_day > 0 && mission.threads_today >= mission.max_threads_per_day {
            debug!(mission_id = %id, "daily thread budget exhausted");
            return Ok(None);
        }

        // Cooldown: refuse to fire if the last successful fire was within
        // `cooldown_secs` of now. 0 = disabled.
        if mission.cooldown_secs > 0
            && let Some(last) = mission.last_fire_at
        {
            let elapsed = chrono::Utc::now().signed_duration_since(last).num_seconds();
            if elapsed >= 0 && (elapsed as u64) < mission.cooldown_secs {
                debug!(
                    mission_id = %id,
                    elapsed_secs = elapsed,
                    cooldown_secs = mission.cooldown_secs,
                    "mission cooldown not yet elapsed"
                );
                return Ok(None);
            }
        }

        // max_concurrent: count threads from this mission that are still in
        // a non-terminal state. 0 = unlimited.
        if mission.max_concurrent > 0 {
            let running = self.count_running_threads(&mission).await;
            if running >= mission.max_concurrent as usize {
                debug!(
                    mission_id = %id,
                    running,
                    max_concurrent = mission.max_concurrent,
                    "mission max_concurrent reached"
                );
                return Ok(None);
            }
        }

        // Per-user global rate limit. Independent of per-mission cooldown,
        // this is a sliding-window cap across *all* of the user's missions
        // so a user with many event-triggered missions can't collectively
        // flood the LLM. We only *check* here — recording is deferred until
        // after the spawn succeeds so a downstream failure (store error,
        // budget refusal, spawn error) doesn't consume a slot and slowly
        // self-DoS the user.
        if !self.check_user_rate(&mission.user_id).await {
            debug!(
                mission_id = %id,
                user_id = %mission.user_id,
                max_fires = self.rate_limit.max_fires,
                window_secs = self.rate_limit.window.as_secs(),
                "per-user mission fire rate limit reached"
            );
            return Ok(None);
        }

        // Budget gate: when the host wires a `BudgetGate` (typically over
        // its CostGuard), refuse to fire when the user is out of budget.
        // Unattached gate = always allow.
        if !self.budget_allows(&mission.user_id, id).await {
            debug!(
                mission_id = %id,
                user_id = %mission.user_id,
                "mission fire refused by budget gate"
            );
            return Ok(None);
        }

        // Load context_paths from the workspace if a reader is attached.
        // Failures are logged but never block the fire — context loading is
        // a best-effort enrichment, not a precondition.
        let mut context_blocks: Vec<(String, String)> = Vec::new();
        if let Some(reader) = self.workspace.as_ref() {
            for path in &mission.context_paths {
                match reader.read_doc(path).await {
                    Ok(content) => context_blocks.push((path.clone(), content)),
                    Err(error) => debug!(
                        mission_id = %id,
                        path = %path,
                        error = %error,
                        "failed to load mission context_path; skipping"
                    ),
                }
            }
        } else if !mission.context_paths.is_empty() {
            debug!(
                mission_id = %id,
                paths = mission.context_paths.len(),
                "mission has context_paths but no WorkspaceReader is attached"
            );
        }

        // Build meta-prompt from mission state + project docs
        let retrieval = RetrievalEngine::new(Arc::clone(&self.store));
        let project_docs = retrieval
            .retrieve_context(mission.project_id, &mission.user_id, &mission.goal, 10)
            .await
            .unwrap_or_default();
        let meta_prompt =
            build_meta_prompt(&mission, &project_docs, &trigger_payload, &context_blocks);

        // Spawn thread with meta-prompt as initial user message
        let thread_id = self
            .thread_manager
            .spawn_thread(
                &meta_prompt,
                ThreadType::Mission,
                mission.project_id,
                ThreadConfig::default(),
                None,
                user_id,
            )
            .await?;

        // Capture the fire instant once and use it for both the persisted
        // `last_fire_at` and the in-memory `last_fire_attempt` map. The two
        // writes MUST share the same value: tick's stale-state detection
        // compares them as equal-or-not to decide whether the cooldown
        // applies, and using two separate `Utc::now()` calls would produce
        // microsecond drift that breaks the equality check on the success
        // path.
        let fire_instant = chrono::Utc::now();

        // Install the outcome watcher *before* persisting the mission update.
        // The watcher only depends on `thread_id` (it joins via ThreadManager
        // and reloads the mission record itself), so installing it first
        // ensures a transient `save_mission` failure below cannot orphan the
        // running thread by skipping the watcher install. Pass `fire_instant`
        // through so the outcome processor can reconcile `last_fire_at`
        // back to the original moment if the save below fails — without
        // this the reconciled value would be the *outcome* time, which can
        // be many seconds-to-hours later for long-running mission threads.
        self.spawn_mission_outcome_watcher(id, thread_id, fire_instant);

        // Record the thread + trigger payload in mission history
        let mut updated = mission;
        let user_id_for_rate = updated.user_id.clone();
        updated.record_thread(thread_id);
        updated.threads_today += 1;
        updated.last_trigger_payload = trigger_payload;
        // Advance next_fire_at for cron missions so the ticker schedules the
        // next cycle. Computed from `now()`, not from the previous fire time:
        // if a tick was delayed (process down, busy loop) and several windows
        // are missed, they coalesce into a single fire here rather than
        // backfilling each missed slot. This is the catch-up semantics we want
        // for long-running missions.
        //
        // Lenient `next_cron_fire` (not `_required`): a parse error here is
        // unlikely (the expression validated at create time) but possible if
        // persisted data is corrupt. We log and preserve the existing
        // `next_fire_at` rather than aborting fire — the thread is already
        // running and the watcher is already installed, and at worst the
        // schedule is delayed by one cycle until the next tick.
        //
        // `cron_advanced` tracks whether scheduling actually progressed. When
        // false (parse error on a corrupt expression), we deliberately leave
        // `last_fire_at` at its OLD persisted value. The in-memory
        // `last_fire_attempt[mid]` will still be set to `fire_instant` below,
        // so the in-memory vs persisted mismatch arms tick's cooldown via
        // the same code path as a save failure. Without this, a corrupt
        // expression with a past `next_fire_at` would re-fire on every tick
        // (cooldown matched, schedule never advanced) and exhaust the daily
        // budget — same root cause as #1944.
        let mut cron_advanced = true;
        if let MissionCadence::Cron {
            ref expression,
            ref timezone,
        } = updated.cadence
        {
            match next_cron_fire(expression, timezone.as_ref()) {
                Ok(next) => updated.next_fire_at = next,
                Err(e) => {
                    cron_advanced = false;
                    debug!(
                        mission_id = %id,
                        expression = %expression,
                        error = %e,
                        "failed to advance next_fire_at after fire; preserving existing value and arming cooldown via mismatch"
                    );
                }
            }
        }
        if cron_advanced {
            updated.last_fire_at = Some(fire_instant);
        }

        // Arm the in-memory cooldown BEFORE the persistence call, not after.
        //
        // Why first: a concurrent tick observing the post-`save_mission`
        // state but pre-`last_fire_attempt`-insert state would see the
        // freshly-persisted `last_fire_at = fire_instant` AND no in-memory
        // entry, evaluate `is_some_and(...)` to false, and (if the schedule
        // is still in the past) re-fire immediately. Inserting first closes
        // that race: while save is in flight, in-memory has `fire_instant`
        // and persisted still has the OLD `last_fire_at`, so tick sees a
        // mismatch and arms the cooldown. Once save lands the values match
        // (success path) or stay mismatched (failure path) — both correct.
        //
        // Held briefly under the write lock; the map is keyed by mission ID
        // and only mutated here, in tick (prune), and in pause/complete.
        self.last_fire_attempt
            .write()
            .await
            .insert(id, fire_instant);

        // Persistence is best-effort: if save_mission fails on a transient store
        // error, the thread is already running and the outcome watcher is already
        // installed (above), so failing here would orphan the work AND — for cron
        // cadences — leave next_fire_at un-advanced, causing the next tick to
        // re-fire the same mission in a runaway loop. Log and continue. The
        // caller still gets Ok(Some(thread_id)) so the spawned thread is visible.
        // The in-memory `last_fire_attempt` cooldown above catches runaway re-fires
        // by comparing in-memory vs persisted `last_fire_at` (see tick).
        if let Err(e) = self.store.save_mission(&updated).await {
            debug!(
                mission_id = %id,
                thread_id = %thread_id,
                error = %e,
                "failed to persist mission update after fire; thread is running and watched, in-memory cooldown will suppress re-fire"
            );
        }

        // Now that the spawn + persist have succeeded, consume a slot in
        // the per-user rate window. Doing this here (rather than at the
        // earlier check site) means store errors, budget refusals, and
        // spawn failures all leave the user's window untouched.
        self.record_user_rate(&user_id_for_rate).await;

        debug!(mission_id = %id, thread_id = %thread_id, "mission fired");

        Ok(Some(thread_id))
    }

    /// Resume suspended checkpointed mission threads after restart.
    pub async fn resume_recoverable_threads(
        &self,
        user_id: &str,
    ) -> Result<Vec<ThreadId>, EngineError> {
        let mut resumed = Vec::new();

        for mission_id in self.active.read().await.clone() {
            let Some(mission) = self.store.load_mission(mission_id).await? else {
                continue;
            };

            for &thread_id in mission.thread_history.iter().rev() {
                let Some(thread) = self.store.load_thread(thread_id).await? else {
                    continue;
                };
                if thread.thread_type != ThreadType::Mission
                    || thread.state != crate::types::thread::ThreadState::Suspended
                {
                    continue;
                }
                if thread.metadata.get("runtime_checkpoint").is_none() {
                    continue;
                }

                self.thread_manager
                    .resume_thread(thread_id, user_id.to_string(), None, None, None)
                    .await?;
                // Resumed threads are already in `thread_history` from the
                // original fire, so the outcome processor will see
                // `needs_reconcile = false` and never read this value. Pass
                // `now` as a safe placeholder; nothing depends on it.
                self.spawn_mission_outcome_watcher(mission_id, thread_id, chrono::Utc::now());
                resumed.push(thread_id);
            }
        }

        Ok(resumed)
    }

    /// Start a background cron ticker that fires due missions every 60 seconds.
    pub fn start_cron_ticker(self: &Arc<Self>, user_id: String) {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                match mgr.tick(&user_id).await {
                    Ok(spawned) if !spawned.is_empty() => {
                        debug!(count = spawned.len(), "cron ticker spawned mission threads");
                    }
                    Err(e) => {
                        debug!("cron ticker error: {e}");
                    }
                    _ => {}
                }
            }
        });
    }

    /// List all missions in a project for a given user.
    /// List missions visible to a user (own + shared).
    pub async fn list_missions(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<Mission>, EngineError> {
        self.store
            .list_missions_with_shared(project_id, user_id)
            .await
    }

    /// Get a mission by ID.
    pub async fn get_mission(&self, id: MissionId) -> Result<Option<Mission>, EngineError> {
        self.store.load_mission(id).await
    }

    /// Fire all active `OnSystemEvent` missions whose source and event_type match.
    ///
    /// The optional `payload` is forwarded as `trigger_payload` to each mission's
    /// thread, carrying context like trace issues and reflection docs.
    pub async fn fire_on_system_event(
        &self,
        source: &str,
        event_type: &str,
        user_id: &str,
        payload: Option<serde_json::Value>,
    ) -> Result<Vec<ThreadId>, EngineError> {
        let active_ids = self.active.read().await.clone();
        let mut spawned = Vec::new();

        for mid in active_ids {
            let mission = match self.store.load_mission(mid).await? {
                Some(m) if m.status == MissionStatus::Active => m,
                _ => continue,
            };

            // Only fire missions owned by this user (per-user learning missions)
            // or globally shared missions.
            if !mission.is_owned_by(user_id) && !mission.owner_id().is_shared() {
                continue;
            }

            let matches = match &mission.cadence {
                MissionCadence::OnSystemEvent {
                    source: s,
                    event_type: et,
                    filters,
                } => {
                    s == source
                        && et == event_type
                        && payload_matches_filters(filters, payload.as_ref())
                }
                _ => false,
            };

            if !matches {
                continue;
            }

            // Dedup: skip if an identical event key fired this mission within
            // its dedup window. The default key is the SHA-256 of the payload
            // serialization (compact and stable for typical webhook bodies).
            if mission.dedup_window_secs > 0 {
                let key = payload_dedup_key(payload.as_ref());
                if self.dedup_event(mid, &key, mission.dedup_window_secs).await {
                    debug!(
                        mission_id = %mid,
                        dedup_window_secs = mission.dedup_window_secs,
                        "skipping system_event fire — dedup window not yet elapsed"
                    );
                    continue;
                }
            }

            if let Some(tid) = self.fire_mission(mid, user_id, payload.clone()).await? {
                spawned.push(tid);
            }
        }

        Ok(spawned)
    }

    /// Fire all active `OnEvent` missions whose `event_pattern` matches
    /// `text` and (if a channel filter is set) whose `channel` matches the
    /// incoming message channel case-insensitively.
    ///
    /// `payload` is forwarded as `trigger_payload` to each mission's thread.
    /// Pattern matching uses simple substring matching to keep this dependency-
    /// free; callers needing regex semantics should normalize first or
    /// extend the matcher.
    pub async fn fire_on_message_event(
        &self,
        channel: &str,
        text: &str,
        user_id: &str,
        payload: Option<serde_json::Value>,
    ) -> Result<Vec<ThreadId>, EngineError> {
        let active_ids = self.active.read().await.clone();
        let mut spawned = Vec::new();

        for mid in active_ids {
            let mission = match self.store.load_mission(mid).await? {
                Some(m) if m.status == MissionStatus::Active => m,
                _ => continue,
            };

            if !mission.is_owned_by(user_id) && !mission.owner_id().is_shared() {
                continue;
            }

            let channel_ok = match &mission.cadence {
                MissionCadence::OnEvent {
                    channel: cadence_channel,
                    ..
                } => cadence_channel
                    .as_ref()
                    .is_none_or(|c| c.eq_ignore_ascii_case(channel)),
                _ => continue,
            };
            if !channel_ok {
                continue;
            }
            // Regex match (with size-limited compile + per-mission cache).
            // The substring fallback used previously was too loose: it
            // matched "the review was requested yesterday" against
            // "review requested" and would flood on busy channels.
            if !self.event_regex_matches(&mission, text).await {
                continue;
            }

            if mission.dedup_window_secs > 0 {
                let key = payload_dedup_key(payload.as_ref());
                if self.dedup_event(mid, &key, mission.dedup_window_secs).await {
                    continue;
                }
            }

            if let Some(tid) = self.fire_mission(mid, user_id, payload.clone()).await? {
                spawned.push(tid);
            }
        }

        Ok(spawned)
    }

    /// Fire the active `Webhook` mission whose registered `path` matches the
    /// incoming webhook path. The bridge layer is responsible for HMAC
    /// validation against `Webhook.secret` *before* calling this; the engine
    /// just routes payloads to mission threads.
    ///
    /// Returns the IDs of any threads spawned.
    pub async fn fire_on_webhook(
        &self,
        webhook_path: &str,
        user_id: &str,
        payload: Option<serde_json::Value>,
    ) -> Result<Vec<ThreadId>, EngineError> {
        let active_ids = self.active.read().await.clone();
        let mut spawned = Vec::new();

        for mid in active_ids {
            let mission = match self.store.load_mission(mid).await? {
                Some(m) if m.status == MissionStatus::Active => m,
                _ => continue,
            };

            if !mission.is_owned_by(user_id) && !mission.owner_id().is_shared() {
                continue;
            }

            let matches = matches!(
                &mission.cadence,
                MissionCadence::Webhook { path, .. } if path == webhook_path
            );

            if !matches {
                continue;
            }

            if mission.dedup_window_secs > 0 {
                let key = payload_dedup_key(payload.as_ref());
                if self.dedup_event(mid, &key, mission.dedup_window_secs).await {
                    continue;
                }
            }

            if let Some(tid) = self.fire_mission(mid, user_id, payload.clone()).await? {
                spawned.push(tid);
            }
        }

        Ok(spawned)
    }

    /// Start a background event listener that fires learning missions when
    /// threads complete.
    ///
    /// Subscribes to the ThreadManager's event broadcast channel and watches
    /// for `StateChanged { to: Done }`. For each completed non-Mission thread:
    ///
    /// 1. **Skill repair** — if an active skill looks stale or incomplete,
    ///    fires `thread_completed_with_skill_gap`
    /// 2. **Error diagnosis** — if trace has issues, fires `thread_completed_with_issues`
    /// 3. **Skill extraction** — if thread succeeded with many steps/actions,
    ///    fires `thread_completed_with_learnings`
    /// 4. **Conversation insights** — after every N threads in a conversation,
    ///    fires `conversation_insights_due`
    pub fn start_event_listener(self: &Arc<Self>, _owner_id: String) {
        let mgr = Arc::clone(self);
        let mut rx = mgr.thread_manager.subscribe_events();

        /// Minimum steps for a thread to be a skill extraction candidate.
        const SKILL_EXTRACTION_MIN_STEPS: usize = 5;
        /// Minimum distinct action executions for skill extraction.
        const SKILL_EXTRACTION_MIN_ACTIONS: usize = 3;
        /// Completed thread interval for conversation insights.
        const CONVERSATION_INSIGHTS_INTERVAL: u32 = 5;

        tokio::spawn(async move {
            // Track completed thread count per conversation for insights trigger.
            let mut conv_thread_counts: std::collections::HashMap<String, u32> =
                std::collections::HashMap::new();

            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let Some(terminal_state) = learning_terminal_state(&event.kind) else {
                            continue;
                        };

                        // Load the completed thread
                        let thread = match mgr.store.load_thread(event.thread_id).await {
                            Ok(Some(t)) => t,
                            _ => continue,
                        };

                        // Skip Mission threads (no recursive self-improvement)
                        if thread.thread_type == ThreadType::Mission {
                            continue;
                        }

                        let trace = crate::executor::trace::build_trace(&thread);
                        // Single pass over events for both skill-repair and
                        // error-diagnosis triggers (avoids repeated iteration
                        // on large event logs).
                        let (error_messages, _observed_actions) =
                            collect_errors_and_actions(&thread);
                        let active_skills = thread.active_skills();

                        // ── Trigger 1: Skill repair ───────────────────────
                        // NOTE: skill-repair and error-diagnosis can both fire
                        // for the same thread. Each targets a different mission
                        // so they won't collide, but both may spawn concurrent
                        // threads. This is intentional — skill-repair fixes the
                        // *skill* while error-diagnosis fixes the *prompt/orchestrator*.
                        if !active_skills.is_empty() {
                            let tracker = SkillTracker::new(Arc::clone(&mgr.store));
                            let success = thread_completed_successfully(&thread, &trace);
                            for skill in &active_skills {
                                if let Err(e) = tracker.record_usage(skill.doc_id, success).await {
                                    debug!(
                                        skill_doc_id = %skill.doc_id.0,
                                        thread_id = %thread.id,
                                        "event listener: failed to record skill usage: {e}"
                                    );
                                }
                            }

                            if let Some(payload) =
                                build_skill_gap_payload(&thread, &trace, &active_skills)
                                && let Err(e) = mgr
                                    .fire_on_system_event(
                                        "engine",
                                        "thread_completed_with_skill_gap",
                                        &thread.user_id,
                                        Some(payload),
                                    )
                                    .await
                            {
                                debug!("event listener: failed to fire skill repair: {e}");
                            }
                        }

                        // ── Trigger 2: Error diagnosis ──────────────────
                        if !trace.issues.is_empty() {
                            let issues: Vec<serde_json::Value> = trace
                                .issues
                                .iter()
                                .map(|i| {
                                    serde_json::json!({
                                        "severity": format!("{:?}", i.severity),
                                        "category": i.category.clone(),
                                        "description": i.description.clone(),
                                        "step": i.step,
                                    })
                                })
                                .collect();

                            let payload = serde_json::json!({
                                "source_thread_id": event.thread_id.0.to_string(),
                                "goal": thread.goal,
                                "issues": issues,
                                "error_messages": error_messages,
                            });

                            if let Err(e) = mgr
                                .fire_on_system_event(
                                    "engine",
                                    "thread_completed_with_issues",
                                    &thread.user_id,
                                    Some(payload),
                                )
                                .await
                            {
                                debug!("event listener: failed to fire error diagnosis: {e}");
                            }
                        }

                        // ── Trigger 3: Skill extraction ──────────────────
                        let action_count = thread
                            .events
                            .iter()
                            .filter(|e| {
                                matches!(
                                    e.kind,
                                    crate::types::event::EventKind::ActionExecuted { .. }
                                )
                            })
                            .count();

                        if terminal_state == crate::types::thread::ThreadState::Done
                            && trace
                                .issues
                                .iter()
                                .all(|i| i.severity != crate::executor::trace::IssueSeverity::Error)
                            && thread.step_count >= SKILL_EXTRACTION_MIN_STEPS
                            && action_count >= SKILL_EXTRACTION_MIN_ACTIONS
                        {
                            let actions_used: Vec<String> = thread
                                .events
                                .iter()
                                .filter_map(|e| {
                                    if let crate::types::event::EventKind::ActionExecuted {
                                        action_name,
                                        ..
                                    } = &e.kind
                                    {
                                        Some(action_name.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect();

                            let payload = serde_json::json!({
                                "source_thread_id": event.thread_id.0.to_string(),
                                "goal": thread.goal,
                                "step_count": thread.step_count,
                                "action_count": action_count,
                                "actions_used": actions_used,
                                "total_tokens": thread.total_tokens_used,
                            });

                            if let Err(e) = mgr
                                .fire_on_system_event(
                                    "engine",
                                    "thread_completed_with_learnings",
                                    &thread.user_id,
                                    Some(payload),
                                )
                                .await
                            {
                                debug!("event listener: failed to fire skill extraction: {e}");
                            }
                        }

                        // ── Trigger 4: Conversation insights ────────────
                        // Keep insights tied to successful completions only.
                        if should_count_for_conversation_insights(terminal_state) {
                            // Use the thread's project_id as a proxy for conversation scope.
                            let conv_key = thread.project_id.0.to_string();
                            let count = conv_thread_counts.entry(conv_key.clone()).or_insert(0);
                            *count += 1;

                            if (*count).is_multiple_of(CONVERSATION_INSIGHTS_INTERVAL) {
                                // Collect recent thread goals for context
                                let thread_goals: Vec<String> = match mgr
                                    .store
                                    .list_threads(thread.project_id, &thread.user_id)
                                    .await
                                {
                                    Ok(threads) => threads
                                        .iter()
                                        .rev()
                                        .take(CONVERSATION_INSIGHTS_INTERVAL as usize)
                                        .map(|t| t.goal.clone())
                                        .collect(),
                                    Err(_) => vec![thread.goal.clone()],
                                };

                                // Collect sample user messages from recent threads
                                let sample_messages: Vec<String> = thread
                                    .messages
                                    .iter()
                                    .filter(|m| m.role == crate::types::message::MessageRole::User)
                                    .map(|m| m.content.chars().take(200).collect::<String>())
                                    .take(10)
                                    .collect();

                                let payload = serde_json::json!({
                                    "project_id": thread.project_id.0.to_string(),
                                    "completed_thread_count": *count,
                                    "thread_goals": thread_goals,
                                    "sample_user_messages": sample_messages,
                                });

                                if let Err(e) = mgr
                                    .fire_on_system_event(
                                        "engine",
                                        "conversation_insights_due",
                                        &thread.user_id,
                                        Some(payload),
                                    )
                                    .await
                                {
                                    debug!(
                                        "event listener: failed to fire conversation insights: {e}"
                                    );
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        debug!("event listener: lagged {n} events");
                    }
                }
            }
        });
    }

    /// Ensure a self-improvement mission exists for the given project.
    ///
    /// Checks if a mission with `"self_improvement": true` in metadata already
    /// exists. If not, creates one with `OnSystemEvent` cadence that fires
    /// when threads complete with issues. Also seeds the fix pattern database.
    ///
    /// Returns the mission ID (existing or newly created).
    pub async fn ensure_self_improvement_mission(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<MissionId, EngineError> {
        // Check if this user already has a self-improvement mission.
        let missions = self.store.list_missions(project_id, user_id).await?;
        if let Some(existing) = missions.iter().find(|m| is_self_improvement_mission(m)) {
            debug!(mission_id = %existing.id, "self-improvement mission already exists");
            // Make sure it's in the active list
            let mut active = self.active.write().await;
            if !active.contains(&existing.id) {
                active.push(existing.id);
            }
            return Ok(existing.id);
        }

        // Create per-user self-improvement mission
        let mut mission = Mission::new(
            project_id,
            user_id,
            "self-improvement",
            SELF_IMPROVEMENT_GOAL,
            MissionCadence::OnSystemEvent {
                source: "engine".into(),
                event_type: "thread_completed_with_issues".into(),
                filters: std::collections::HashMap::new(),
            },
        );
        mission.success_criteria = Some(
            "Continuously improve system prompts and fix patterns based on execution traces".into(),
        );
        mission.metadata = serde_json::json!({"self_improvement": true});
        mission.max_threads_per_day = 5;
        // Future-proof: today this helper only ever uses OnSystemEvent, but if
        // a future caller passes a Cron cadence the same `next_fire_at = None`
        // bug that #1944 fixed in `create_mission` would silently re-emerge
        // here. Compute next_fire_at on construction so this helper can never
        // produce a stuck cron mission.
        if let MissionCadence::Cron {
            ref expression,
            ref timezone,
        } = mission.cadence
        {
            mission.next_fire_at = Some(next_cron_fire_required(expression, timezone.as_ref())?);
        }

        let id = mission.id;
        self.store.save_mission(&mission).await?;
        self.active.write().await.push(id);

        // Seed the fix pattern database if it doesn't exist
        let docs = self.store.list_shared_memory_docs(project_id).await?;
        let has_patterns = docs.iter().any(|d| {
            d.title == FIX_PATTERN_DB_TITLE && d.tags.contains(&FIX_PATTERN_DB_TAG.to_string())
        });
        if !has_patterns {
            use crate::types::memory::{DocType, MemoryDoc};
            let pattern_doc = MemoryDoc::new(
                project_id,
                shared_owner_id(),
                DocType::Note,
                FIX_PATTERN_DB_TITLE,
                SEED_FIX_PATTERNS,
            )
            .with_tags(vec![FIX_PATTERN_DB_TAG.to_string()]);
            self.store.save_memory_doc(&pattern_doc).await?;
            debug!("seeded fix pattern database");
        }

        debug!(mission_id = %id, "created self-improvement mission");
        Ok(id)
    }

    /// Ensure the built-in learning missions exist for the given project.
    ///
    /// Creates (if missing) the self-improvement, skill repair, skill
    /// extraction, and conversation insights missions. This is the preferred
    /// entry point — call once at project bootstrap.
    pub async fn ensure_learning_missions(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<(), EngineError> {
        // 0. Seed compiled-in orchestrator v0 so it's visible in workspace
        self.seed_orchestrator_v0(project_id).await?;

        // 1. Error diagnosis (self-improvement) — per-user
        self.ensure_self_improvement_mission(project_id, user_id)
            .await?;

        // 2. Skill repair
        self.ensure_mission_by_metadata(
            project_id,
            user_id,
            "skill_repair",
            "skill-repair",
            SKILL_REPAIR_GOAL,
            MissionCadence::OnSystemEvent {
                source: "engine".into(),
                event_type: "thread_completed_with_skill_gap".into(),
                filters: HashMap::new(),
            },
            "Repair versioned skills when execution reveals stale or incomplete instructions",
            5,
        )
        .await?;

        // 3. Skill extraction (formerly playbook extraction)
        self.ensure_mission_by_metadata(
            project_id,
            user_id,
            "skill_extraction",
            "skill-extraction",
            SKILL_EXTRACTION_GOAL,
            MissionCadence::OnSystemEvent {
                source: "engine".into(),
                event_type: "thread_completed_with_learnings".into(),
                filters: std::collections::HashMap::new(),
            },
            "Extract reusable skills from successful multi-step threads",
            3, // max 3/day
        )
        .await?;

        // 4. Conversation insights
        self.ensure_mission_by_metadata(
            project_id,
            user_id,
            "conversation_insights",
            "conversation-insights",
            CONVERSATION_INSIGHTS_GOAL,
            MissionCadence::OnSystemEvent {
                source: "engine".into(),
                event_type: "conversation_insights_due".into(),
                filters: std::collections::HashMap::new(),
            },
            "Extract user preferences, domain knowledge, and workflow patterns from conversations",
            2, // max 2/day
        )
        .await?;

        // 5. Expected behavior (user feedback loop)
        self.ensure_mission_by_metadata(
            project_id,
            user_id,
            "expected_behavior",
            "expected-behavior",
            EXPECTED_BEHAVIOR_GOAL,
            MissionCadence::OnSystemEvent {
                source: "user_feedback".into(),
                event_type: "expected_behavior".into(),
                filters: std::collections::HashMap::new(),
            },
            "Investigate user-reported expectation gaps and apply fixes",
            5, // max 5/day
        )
        .await?;

        Ok(())
    }

    /// Seed the compiled-in orchestrator as v0 in the Store.
    ///
    /// This makes v0 visible in the workspace memory tree and provides a base
    /// for the self-improvement mission to diff against when patching. If the
    /// compiled-in code has changed (different content hash), the stored v0 is
    /// updated to match — runtime patches (v1+) are left untouched.
    async fn seed_orchestrator_v0(&self, project_id: ProjectId) -> Result<(), EngineError> {
        use crate::executor::orchestrator::{
            DEFAULT_ORCHESTRATOR, ORCHESTRATOR_TAG, ORCHESTRATOR_TITLE,
        };
        use crate::types::memory::{DocType, MemoryDoc};

        let docs = self.store.list_shared_memory_docs(project_id).await?;
        let existing_v0 = docs.iter().find(|d| {
            d.title == ORCHESTRATOR_TITLE
                && d.tags.contains(&ORCHESTRATOR_TAG.to_string())
                && d.metadata
                    .get("version")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
                    == 0
        });

        if let Some(doc) = existing_v0 {
            // Update if compiled-in code changed (rebuild with new default.py)
            if doc.content != DEFAULT_ORCHESTRATOR {
                let mut updated = doc.clone();
                updated.content = DEFAULT_ORCHESTRATOR.to_string();
                updated.updated_at = chrono::Utc::now();
                self.store.save_memory_doc(&updated).await?;
                debug!("updated orchestrator v0 to match compiled-in default");
            }
            return Ok(());
        }

        // Create v0 doc
        let mut doc = MemoryDoc::new(
            project_id,
            shared_owner_id(),
            DocType::Note,
            ORCHESTRATOR_TITLE,
            DEFAULT_ORCHESTRATOR,
        )
        .with_tags(vec![ORCHESTRATOR_TAG.to_string()]);
        doc.metadata = serde_json::json!({"version": 0, "source": "compiled_in"});
        self.store.save_memory_doc(&doc).await?;
        debug!("seeded orchestrator v0 in workspace");
        Ok(())
    }

    /// Ensure a mission with a specific metadata tag exists, creating it if not.
    #[allow(clippy::too_many_arguments)]
    async fn ensure_mission_by_metadata(
        &self,
        project_id: ProjectId,
        user_id: &str,
        metadata_key: &str,
        name: &str,
        goal: &str,
        cadence: MissionCadence,
        success_criteria: &str,
        max_per_day: u32,
    ) -> Result<MissionId, EngineError> {
        // Check if this user already has a mission with this metadata key.
        let missions = self.store.list_missions(project_id, user_id).await?;
        if let Some(existing) = missions
            .iter()
            .find(|m| m.metadata.get(metadata_key).is_some())
        {
            let mut active = self.active.write().await;
            if !active.contains(&existing.id) {
                active.push(existing.id);
            }
            return Ok(existing.id);
        }

        let mut mission = Mission::new(project_id, user_id, name, goal, cadence);
        mission.success_criteria = Some(success_criteria.into());
        mission.metadata = serde_json::json!({metadata_key: true});
        mission.max_threads_per_day = max_per_day;
        // Future-proof: today every caller passes OnSystemEvent, but a future
        // caller passing Cron would silently re-introduce the `next_fire_at =
        // None` bug that #1944 fixed in `create_mission`. Compute it here so
        // this helper can never produce a stuck cron mission.
        if let MissionCadence::Cron {
            ref expression,
            ref timezone,
        } = mission.cadence
        {
            mission.next_fire_at = Some(next_cron_fire_required(expression, timezone.as_ref())?);
        }

        let id = mission.id;
        self.store.save_mission(&mission).await?;
        self.active.write().await.push(id);

        debug!(mission_id = %id, name, "created learning mission");
        Ok(id)
    }

    /// Tick — check all active missions and fire any that are due.
    ///
    /// For `Cron` cadence missions, checks `next_fire_at` against current time.
    /// For `Manual` missions, this is a no-op.
    /// Returns the IDs of threads spawned.
    pub async fn tick(&self, _fallback_user_id: &str) -> Result<Vec<ThreadId>, EngineError> {
        let active_ids = self.active.read().await.clone();
        let mut spawned = Vec::new();
        let now = chrono::Utc::now();
        let cooldown =
            chrono::Duration::from_std(FIRE_COOLDOWN).unwrap_or(chrono::Duration::zero());

        // Opportunistic prune of `last_fire_attempt`: drop entries whose
        // cooldown window has already elapsed. This catches stragglers from
        // missions that were removed without going through the graceful
        // pause/complete paths (e.g. crash recovery, direct store edits) so
        // the map can never grow unbounded over a long-lived process.
        {
            let mut map = self.last_fire_attempt.write().await;
            map.retain(|_, last| now.signed_duration_since(*last) < cooldown);
        }

        for mid in active_ids {
            // Per-mission error isolation: a transient store/load error or a
            // single fire failure must not abort the entire tick — the other
            // active missions still need their chance to fire on this cycle.
            let mission = match self.store.load_mission(mid).await {
                Ok(Some(m)) if m.status == MissionStatus::Active => m,
                Ok(_) => continue,
                Err(e) => {
                    debug!(mission_id = %mid, error = %e, "tick: failed to load mission; skipping");
                    continue;
                }
            };

            let should_fire = match &mission.cadence {
                MissionCadence::Cron { .. } => {
                    // Fire if next_fire_at has passed
                    mission.next_fire_at.is_some_and(|next| next <= now)
                }
                MissionCadence::Manual => false,
                MissionCadence::OnEvent { .. }
                | MissionCadence::OnSystemEvent { .. }
                | MissionCadence::Webhook { .. } => false,
            };

            if !should_fire {
                continue;
            }

            // In-memory cooldown — only armed when we can prove
            // `fire_mission`'s post-spawn state didn't make scheduling
            // progress. The detection: `fire_mission` writes the **same**
            // instant to both `last_fire_attempt[mid]` (always, before
            // save_mission) and the persisted `Mission.last_fire_at` (only
            // when both `save_mission` succeeds *and* cron advance
            // succeeded). The two paths that leave the values *unequal*:
            //
            //  - `save_mission` failed → persisted `last_fire_at` still
            //    holds the OLD value (or None).
            //  - The cron expression couldn't be parsed (corrupt
            //    persisted state) → fire_mission deliberately leaves
            //    `last_fire_at` at the OLD value so this same mismatch
            //    arms the cooldown without inventing a new signal.
            //
            // On the success path the two values match and tick treats
            // the cooldown as transparent — a normally-firing every-minute
            // cron passes through the check regardless of how short the
            // schedule is. Once the outcome processor reconciles
            // `last_fire_at` back to the in-memory instant the mismatch
            // resolves even before the 90 s window elapses.
            //
            // **Precision requirement (load-bearing):** the equality check
            // requires the `Store` implementation to round-trip
            // `DateTime<Utc>` without precision loss. The bridge's
            // `HybridStoreAdapter::load_mission` returns from an in-memory
            // `HashMap<MissionId, Mission>` cache populated by `save_mission`,
            // so it preserves nanosecond precision. JSON persistence via
            // `serde` also preserves nanoseconds via RFC3339. A future
            // backend that truncates timestamps (e.g. PostgreSQL TIMESTAMPTZ
            // → microseconds) would silently break the success-path detection
            // and arm the cooldown on every fire — the comparison would need
            // to be relaxed to "within one microsecond" before that lands.
            let on_cooldown =
                self.last_fire_attempt
                    .read()
                    .await
                    .get(&mid)
                    .is_some_and(|in_mem_last| {
                        now.signed_duration_since(*in_mem_last) < cooldown
                            && mission.last_fire_at != Some(*in_mem_last)
                    });
            if on_cooldown {
                debug!(
                    mission_id = %mid,
                    "tick: detected stale persisted last_fire_at after fire; suppressing re-fire until reconcile"
                );
                continue;
            }

            // Per-mission error isolation: a single fire failure must not
            // abort the entire tick — the other active missions still need
            // their chance on this cycle. `fire_mission` enforces
            // `cooldown_secs` and `max_concurrent` independently of the cron
            // next_fire_at, so a cron mission whose schedule fires faster
            // than its cooldown will simply skip the intervening firings
            // rather than backlog them. Cron missions are fired with the
            // mission's own user_id so artifacts stay tenant-scoped.
            match self.fire_mission(mid, &mission.user_id, None).await {
                Ok(Some(tid)) => spawned.push(tid),
                Ok(None) => {}
                Err(e) => debug!(
                    mission_id = %mid,
                    error = %e,
                    "tick: fire_mission failed; continuing with remaining missions"
                ),
            }
        }

        Ok(spawned)
    }

    /// Count threads spawned by `mission` that are still in a non-terminal
    /// state (anything other than `Done`/`Failed`). Used by `max_concurrent`
    /// enforcement. Walks the in-memory thread cache; threads that the store
    /// no longer knows about are treated as terminal.
    async fn count_running_threads(&self, mission: &Mission) -> usize {
        let mut running = 0;
        for tid in mission.thread_history.iter().rev() {
            match self.store.load_thread(*tid).await {
                Ok(Some(thread)) => {
                    if !matches!(thread.state, ThreadState::Done | ThreadState::Failed) {
                        running += 1;
                    }
                }
                _ => continue,
            }
        }
        running
    }

    /// Returns `true` if `(mission_id, dedup_key)` was last seen within
    /// `window_secs`. Updates the table to record `now` for the next call.
    ///
    /// Eviction is done **per entry** against this mission's own window —
    /// never globally — because different missions can have different
    /// `dedup_window_secs` values. A previous implementation called
    /// `table.retain` with the current mission's window across the whole
    /// table, which would silently drop fresh entries belonging to a
    /// longer-window mission and cause duplicate firings.
    async fn dedup_event(&self, mission_id: MissionId, dedup_key: &str, window_secs: u64) -> bool {
        if window_secs == 0 {
            return false;
        }
        let now = chrono::Utc::now();
        let window = chrono::Duration::seconds(window_secs as i64);
        let mut table = self.dedup_table.write().await;
        let key = (mission_id, dedup_key.to_string());
        match table.get(&key) {
            Some(last) if now.signed_duration_since(*last) < window => {
                // Within this mission's own window — duplicate.
                true
            }
            _ => {
                // Either no entry, or the entry has aged past this
                // mission's window. Overwrite (or insert) and report
                // first-seen. We deliberately do NOT touch entries for
                // other missions.
                table.insert(key, now);
                false
            }
        }
    }

    /// Test whether `text` matches `mission`'s OnEvent regex. Compiles the
    /// pattern lazily on first match attempt and caches it. Patterns that
    /// fail to compile (or exceed `MAX_EVENT_REGEX_SIZE`) are logged at
    /// warn level and never match.
    async fn event_regex_matches(&self, mission: &Mission, text: &str) -> bool {
        let MissionCadence::OnEvent { event_pattern, .. } = &mission.cadence else {
            return false;
        };

        // Cache hit fast path.
        if let Some(re) = self.event_regex_cache.read().await.get(&mission.id) {
            return re.is_match(text);
        }

        // Compile under the write lock and double-check (another caller may
        // have raced ahead and inserted the same key).
        let mut cache = self.event_regex_cache.write().await;
        if let Some(re) = cache.get(&mission.id) {
            return re.is_match(text);
        }
        match regex::RegexBuilder::new(event_pattern)
            .size_limit(MAX_EVENT_REGEX_SIZE)
            .build()
        {
            Ok(re) => {
                let matches = re.is_match(text);
                cache.insert(mission.id, re);
                matches
            }
            Err(error) => {
                warn!(
                    mission_id = %mission.id,
                    pattern = %event_pattern,
                    error = %error,
                    "OnEvent mission regex failed to compile (or exceeded size limit); refusing to match"
                );
                false
            }
        }
    }

    /// Drop the compiled regex for `mission_id`, forcing recompile on the
    /// next match attempt. Called when a mission's cadence changes or it is
    /// deleted.
    async fn evict_event_regex(&self, mission_id: MissionId) {
        self.event_regex_cache.write().await.remove(&mission_id);
    }

    /// Per-user global rate limiter — read-only check. Sliding window of
    /// timestamps; returns `true` if a new fire is currently allowed.
    /// Evicts expired entries from the user's window as a side effect, but
    /// does NOT record a new entry — call [`record_user_rate`] only after
    /// the fire has actually succeeded so a failed spawn cannot consume a
    /// slot (otherwise sustained store errors would self-DoS the user).
    ///
    /// [`record_user_rate`]: Self::record_user_rate
    async fn check_user_rate(&self, user_id: &str) -> bool {
        let now = chrono::Utc::now();
        let window = chrono::Duration::from_std(self.rate_limit.window)
            .unwrap_or_else(|_| chrono::Duration::seconds(self.rate_limit.window.as_secs() as i64));
        let cutoff = now - window;

        let mut log = self.user_fire_log.write().await;
        let entries = log.entry(user_id.to_string()).or_default();
        while entries.front().is_some_and(|ts| *ts < cutoff) {
            entries.pop_front();
        }
        (entries.len() as u32) < self.rate_limit.max_fires
    }

    /// Record a successful fire against the per-user rate window. Pair with
    /// [`check_user_rate`] — call only after the spawn has actually
    /// completed so failed fires don't consume a slot.
    ///
    /// [`check_user_rate`]: Self::check_user_rate
    async fn record_user_rate(&self, user_id: &str) {
        let now = chrono::Utc::now();
        let mut log = self.user_fire_log.write().await;
        let entries = log.entry(user_id.to_string()).or_default();
        entries.push_back(now);
    }

    /// Consult the budget gate (if attached). Returns `true` when the gate
    /// is unattached or explicitly allows the fire.
    async fn budget_allows(&self, user_id: &str, mission_id: MissionId) -> bool {
        match self.budget_gate.as_ref() {
            Some(gate) => gate.allow_mission_fire(user_id, mission_id).await,
            None => true,
        }
    }

    fn spawn_mission_outcome_watcher(
        &self,
        mission_id: MissionId,
        thread_id: ThreadId,
        original_fire_at: chrono::DateTime<chrono::Utc>,
    ) {
        let tm = Arc::clone(&self.thread_manager);
        let store = Arc::clone(&self.store);
        let notification_tx = self.notification_tx.clone();
        tokio::spawn(async move {
            match tm.join_thread(thread_id).await {
                Ok(outcome) => {
                    if let Err(e) = process_mission_outcome_and_notify(
                        &store,
                        mission_id,
                        thread_id,
                        &outcome,
                        &notification_tx,
                        Some(original_fire_at),
                    )
                    .await
                    {
                        debug!(mission_id = %mission_id, "failed to process outcome: {e}");
                    }
                }
                Err(e) => {
                    debug!(mission_id = %mission_id, "thread join failed: {e}");
                }
            }
        });
    }
}

// ── Meta-prompt generation ───────────────────────────────────

/// Build the meta-prompt for a mission thread.
///
/// Assembles the mission's goal, current focus, approach history, and
/// relevant project docs into a structured prompt that guides the thread.
/// Returns `true` if every `(key, value)` pair in `filters` matches the
/// payload's top-level field exactly. An empty filter map always matches.
/// `None` payload only matches an empty filter map.
fn payload_matches_filters(
    filters: &HashMap<String, serde_json::Value>,
    payload: Option<&serde_json::Value>,
) -> bool {
    if filters.is_empty() {
        return true;
    }
    let Some(payload) = payload else {
        return false;
    };
    let Some(obj) = payload.as_object() else {
        return false;
    };
    filters
        .iter()
        .all(|(key, expected)| obj.get(key).is_some_and(|actual| actual == expected))
}

/// Compute a stable dedup key for an event payload. Hashes the canonicalized
/// JSON serialization with the standard library hasher (non-cryptographic but
/// sufficient for in-memory dedup of trusted host-sourced events). Empty/None
/// payloads collapse to a single fixed key so a flood of identical empty
/// events is suppressed.
fn payload_dedup_key(payload: Option<&serde_json::Value>) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let serialized = match payload {
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
        None => String::new(),
    };
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn build_meta_prompt(
    mission: &Mission,
    project_docs: &[MemoryDoc],
    trigger_payload: &Option<serde_json::Value>,
    context_blocks: &[(String, String)],
) -> String {
    let mut parts = Vec::new();

    parts.push(format!(
        "# Mission: {}\n\nGoal: {}",
        mission.name, mission.goal
    ));

    if let Some(description) = &mission.description {
        parts.push(format!("\n{description}"));
    }

    if let Some(criteria) = &mission.success_criteria {
        parts.push(format!("Success criteria: {criteria}"));
    }

    // Preloaded workspace context (`Mission.context_paths`).
    if !context_blocks.is_empty() {
        parts.push("\n## Loaded Context".into());
        for (path, content) in context_blocks {
            parts.push(format!("### {path}\n\n{content}"));
        }
    }

    // Current focus
    if let Some(focus) = &mission.current_focus {
        parts.push(format!("\n## Current Focus\n{focus}"));
    } else if mission.thread_history.is_empty() {
        parts.push("\n## Current Focus\nThis is the first run. Start by understanding the goal and determining the first step.".into());
    }

    // Approach history
    if !mission.approach_history.is_empty() {
        parts.push("\n## Previous Approaches".into());
        for (i, approach) in mission.approach_history.iter().enumerate() {
            parts.push(format!("{}. {approach}", i + 1));
        }
    }

    // Project knowledge (from reflection docs)
    if !project_docs.is_empty() {
        parts.push("\n## Knowledge from Prior Threads".into());
        for doc in project_docs {
            let label = format!("{:?}", doc.doc_type).to_uppercase();
            let content: String = doc.content.chars().take(500).collect();
            let truncated = if doc.content.chars().count() > 500 {
                "..."
            } else {
                ""
            };
            parts.push(format!("[{label}] {}: {content}{truncated}", doc.title));
        }
    }

    // Trigger payload
    if let Some(payload) = trigger_payload {
        let payload_str = serde_json::to_string_pretty(payload).unwrap_or_default();
        let preview: String = payload_str.chars().take(1000).collect();
        parts.push(format!("\n## Trigger Payload\n```json\n{preview}\n```"));
    }

    // Thread count
    parts.push(format!(
        "\nThis is thread #{} for this mission.",
        mission.thread_history.len() + 1
    ));

    // Instructions
    parts.push(
        "\n## Instructions\nBased on the above context, take the next step toward the goal. \
Use tools to gather information, analyze data, or take actions. \
When done, call FINAL() with your response. Include:\n\
1. What you accomplished in this step\n\
2. What the next focus should be (for the next thread)\n\
3. Whether the goal has been achieved (yes/no)"
            .into(),
    );

    parts.join("\n")
}

/// Process a completed mission thread's outcome.
///
/// Extracts next_focus from the FINAL() response and updates the mission.
/// For self-improvement missions (metadata contains `"self_improvement": true`),
/// also processes prompt overlay additions and fix pattern updates.
#[cfg(test)]
async fn process_mission_outcome(
    store: &Arc<dyn Store>,
    mission_id: MissionId,
    thread_id: ThreadId,
    outcome: &ThreadOutcome,
) -> Result<(), EngineError> {
    let (notification_tx, _) = tokio::sync::broadcast::channel(1);
    process_mission_outcome_and_notify(
        store,
        mission_id,
        thread_id,
        outcome,
        &notification_tx,
        None,
    )
    .await
}

async fn process_mission_outcome_and_notify(
    store: &Arc<dyn Store>,
    mission_id: MissionId,
    thread_id: ThreadId,
    outcome: &ThreadOutcome,
    notification_tx: &tokio::sync::broadcast::Sender<MissionNotification>,
    original_fire_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), EngineError> {
    let mut mission = match store.load_mission(mission_id).await? {
        Some(m) => m,
        None => return Ok(()),
    };

    // Reconcile fire-accounting fields that `fire_mission` failed to persist.
    //
    // `fire_mission` is best-effort about its post-spawn `save_mission`: a
    // transient store error there leaves the persisted mission missing this
    // thread's record (`thread_history`, `threads_today`, `last_fire_at`,
    // and — for cron cadences — the advanced `next_fire_at`). The in-memory
    // `last_fire_attempt` cooldown holds the runaway-re-fire path closed
    // for ~90 s, but once the cooldown elapses tick would otherwise re-fire
    // against the stale persisted state. The outcome processor is the
    // natural reconciliation point: by the time we run, the thread has
    // completed and we know exactly which `thread_id` should be present.
    // Append idempotently — repeated invocations or replays are safe — and
    // immediately overwrite our save below, achieving eventual consistency
    // for transient store failures even after retries are exhausted.
    let needs_reconcile = !mission.thread_history.contains(&thread_id);
    if needs_reconcile {
        debug!(
            mission_id = %mission_id,
            thread_id = %thread_id,
            "outcome processor: reconciling fire-accounting fields missing from persisted mission (fire_mission save likely failed)"
        );
        // `record_thread` also bumps `updated_at`; matches the fire_mission
        // path so the two routes don't diverge on field-mutation patterns.
        mission.record_thread(thread_id);
        mission.threads_today = mission.threads_today.saturating_add(1);
        // Reconcile `last_fire_at` back to the **original** fire instant
        // when we know it (passed through from fire_mission via the
        // outcome watcher). Otherwise fall back to `now` as a conservative
        // approximation. Using the original instant matters for users with
        // a configured `cooldown_secs`: if the thread ran for N seconds,
        // a `now`-based reconcile would extend the user's cooldown window
        // by N, gradually drifting the schedule.
        mission.last_fire_at = Some(original_fire_at.unwrap_or_else(chrono::Utc::now));
        // For cron missions, advance `next_fire_at` so the ticker doesn't
        // immediately re-fire against the stale schedule. Use the lenient
        // `next_cron_fire` (not `_required`) — a corrupt expression here
        // should not block the outcome save.
        if let MissionCadence::Cron {
            ref expression,
            ref timezone,
        } = mission.cadence
        {
            let now = chrono::Utc::now();
            let needs_advance = mission.next_fire_at.is_none_or(|next| next <= now);
            if needs_advance {
                match next_cron_fire(expression, timezone.as_ref()) {
                    Ok(Some(next)) => mission.next_fire_at = Some(next),
                    Ok(None) => debug!(
                        mission_id = %mission_id,
                        expression = %expression,
                        "reconcile: cron has no upcoming fire time; leaving next_fire_at unset"
                    ),
                    Err(e) => debug!(
                        mission_id = %mission_id,
                        expression = %expression,
                        error = %e,
                        "reconcile: failed to recompute next_fire_at; leaving as-is"
                    ),
                }
            }
        }
    }

    // Build notification fields while processing the outcome.
    let mut notify_response: Option<String> = None;
    let mut is_error = false;

    match outcome {
        ThreadOutcome::Completed {
            response: Some(text),
        } => {
            // Try to extract next focus and goal status from the response
            let lower = text.to_lowercase();

            // Check if goal achieved
            if lower.contains("goal has been achieved: yes")
                || lower.contains("goal achieved: yes")
                || lower.contains("mission complete")
            {
                debug!(mission_id = %mission_id, "goal achieved — completing mission");
                mission.status = MissionStatus::Completed;
            }

            // Extract next focus (look for "next focus:" pattern)
            if let Some(focus_start) = lower.find("next focus:") {
                let after = &text[focus_start + "next focus:".len()..];
                let next_focus: String = after.lines().next().unwrap_or("").trim().to_string();
                if !next_focus.is_empty() {
                    mission.current_focus = Some(next_focus);
                }
            }

            // Record approach (full response — LLM output is never truncated)
            mission.approach_history.push(text.clone());
            notify_response = Some(text.clone());

            // If this is a self-improvement mission, process structured output
            if is_self_improvement_mission(&mission)
                && let Err(e) = process_self_improvement_output(store, &mission, text).await
            {
                debug!(
                    mission_id = %mission_id,
                    "failed to process self-improvement output: {e}"
                );
            }

            if is_skill_repair_mission(&mission)
                && let Err(e) = process_skill_repair_output(store, &mission, text).await
            {
                debug!(
                    mission_id = %mission_id,
                    "failed to process skill-repair output: {e}"
                );
            }
        }
        ThreadOutcome::Completed { response: None } => {}
        ThreadOutcome::Failed { error } => {
            mission.approach_history.push(format!("FAILED: {error}"));
            notify_response = Some(format!("Mission failed: {error}"));
            is_error = true;
        }
        ThreadOutcome::MaxIterations => {
            mission
                .approach_history
                .push("Hit max iterations without completing".into());
            notify_response = Some("Mission thread hit max iterations without completing".into());
            is_error = true;
        }
        _ => {}
    }

    // Emit notification if there are channels to notify.
    if !mission.notify_channels.is_empty() && notify_response.is_some() {
        // Truncate before broadcasting. Mission threads can produce
        // arbitrarily long output (especially full-job missions); a multi-MB
        // notification is unusable in any chat surface and can OOM Slack/
        // Discord adapters that buffer outbound bodies. The full text is
        // already preserved untruncated in `mission.approach_history`.
        let response = notify_response.map(|text| truncate_notification_text(&text));
        let notification = MissionNotification {
            mission_id,
            mission_name: mission.name.clone(),
            thread_id,
            user_id: mission.user_id.clone(),
            notify_channels: mission.notify_channels.clone(),
            notify_user: mission.notify_user.clone(),
            response,
            is_error,
        };
        // Best-effort: ignore send errors (no subscribers = no problem).
        let _ = notification_tx.send(notification);
    }

    mission.updated_at = chrono::Utc::now();
    store.save_mission(&mission).await
}

/// UTF-8-safe ellipsis truncation for mission notification responses.
///
/// Mirrors the v1 routine engine's `truncate` helper (which uses
/// `floor_char_boundary` from the host `util` module). The engine crate
/// has no `util` so we inline a small helper. The full response text is
/// always preserved untruncated in `Mission.approach_history`; truncation
/// here only affects what is broadcast to notify_channels.
const MAX_NOTIFICATION_RESPONSE_BYTES: usize = 4000;

fn truncate_notification_text(text: &str) -> String {
    if text.len() <= MAX_NOTIFICATION_RESPONSE_BYTES {
        return text.to_string();
    }
    // Walk back from the byte cap to the nearest char boundary so we
    // never split a multi-byte UTF-8 sequence. `is_char_boundary(0)`
    // is always true so the loop is bounded.
    let mut end = MAX_NOTIFICATION_RESPONSE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end]) // safety: end walked back to a valid char boundary above
}

/// Check if a mission is the self-improvement mission.
fn is_self_improvement_mission(mission: &Mission) -> bool {
    mission
        .metadata
        .get("self_improvement")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Check if a mission is the skill-repair mission.
fn is_skill_repair_mission(mission: &Mission) -> bool {
    mission
        .metadata
        .get("skill_repair")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Process output from a self-improvement mission thread.
///
/// Two paths:
/// 1. The agent used tools directly (memory_write for prompt overlay, shell for
///    code fixes) — in this case the FINAL() response is just a summary and
///    there is nothing extra to do here.
/// 2. The agent returned structured JSON with `prompt_additions` and/or
///    `fix_patterns` — we apply those to the Store.
///
/// This function handles path 2. Path 1 is handled by the tools themselves.
async fn process_self_improvement_output(
    store: &Arc<dyn Store>,
    mission: &Mission,
    response: &str,
) -> Result<(), EngineError> {
    use crate::executor::prompt::{PREAMBLE_OVERLAY_TITLE, PROMPT_OVERLAY_TAG};
    use crate::types::memory::{DocType, MemoryDoc};

    // Try to extract JSON from the response. If the agent used tools directly
    // (the preferred autoresearch-style path), there's no JSON and we return
    // early — the work was already done via tool calls.
    let json_val = match extract_json_from_response(response) {
        Some(v) => v,
        None => {
            debug!(
                "self-improvement: no structured JSON in response (agent likely used tools directly)"
            );
            return Ok(());
        }
    };

    let project_id = mission.project_id;

    // Check if self-modification is allowed before applying prompt/orchestrator changes
    let allow_self_modify = std::env::var("ORCHESTRATOR_SELF_MODIFY")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    // Process prompt additions
    if let Some(additions) = json_val.get("prompt_additions").and_then(|v| v.as_array())
        && !additions.is_empty()
    {
        if !allow_self_modify {
            debug!(
                "self-improvement: skipping prompt additions — ORCHESTRATOR_SELF_MODIFY is disabled"
            );
            return Ok(());
        }

        let new_rules: Vec<String> = additions
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        if !new_rules.is_empty() {
            // Load or create the prompt overlay doc
            let docs = store.list_shared_memory_docs(project_id).await?;
            let existing = docs.iter().find(|d| {
                d.title == PREAMBLE_OVERLAY_TITLE
                    && d.tags.contains(&PROMPT_OVERLAY_TAG.to_string())
            });

            let mut overlay = if let Some(doc) = existing {
                doc.clone()
            } else {
                MemoryDoc::new(
                    project_id,
                    shared_owner_id(),
                    DocType::Note,
                    PREAMBLE_OVERLAY_TITLE,
                    "",
                )
                .with_tags(vec![PROMPT_OVERLAY_TAG.to_string()])
            };

            // Append new rules
            for rule in &new_rules {
                if !overlay.content.is_empty() {
                    overlay.content.push('\n');
                }
                overlay.content.push_str(rule);
            }
            overlay.updated_at = chrono::Utc::now();

            store.save_memory_doc(&overlay).await?;
            debug!(
                rules_added = new_rules.len(),
                "self-improvement: updated prompt overlay"
            );
        }
    }

    // Process fix patterns
    if let Some(patterns) = json_val.get("fix_patterns").and_then(|v| v.as_array())
        && !patterns.is_empty()
    {
        let docs = store.list_shared_memory_docs(project_id).await?;
        let existing = docs.iter().find(|d| {
            d.title == FIX_PATTERN_DB_TITLE && d.tags.contains(&FIX_PATTERN_DB_TAG.to_string())
        });

        let mut pattern_doc = if let Some(doc) = existing {
            doc.clone()
        } else {
            MemoryDoc::new(
                project_id,
                shared_owner_id(),
                DocType::Note,
                FIX_PATTERN_DB_TITLE,
                SEED_FIX_PATTERNS,
            )
            .with_tags(vec![FIX_PATTERN_DB_TAG.to_string()])
        };

        for pattern in patterns {
            let p = pattern
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let s = pattern
                .get("strategy")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let l = pattern
                .get("location")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !p.is_empty() {
                pattern_doc
                    .content
                    .push_str(&format!("\n| {p} | {s} | {l} |"));
            }
        }
        pattern_doc.updated_at = chrono::Utc::now();

        store.save_memory_doc(&pattern_doc).await?;
        debug!(
            patterns_added = patterns.len(),
            "self-improvement: updated fix pattern database"
        );
    }

    Ok(())
}

/// Try to extract a JSON object from a response string.
///
/// Looks for `{...}` in the text, trying the whole string first,
/// then searching for embedded JSON.
fn extract_json_from_response(response: &str) -> Option<serde_json::Value> {
    // Try parsing the whole response as JSON
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(response)
        && v.is_object()
    {
        return Some(v);
    }

    // Search for embedded JSON object
    let start = response.find('{')?;
    let end = response.rfind('}')?;
    if end <= start {
        return None;
    }
    let candidate = &response[start..=end];
    serde_json::from_str::<serde_json::Value>(candidate)
        .ok()
        .filter(|v| v.is_object())
}

#[derive(Debug, Deserialize)]
struct SkillRepairMissionOutput {
    doc_id: DocId,
    repair_type: SkillRepairType,
    updated_content: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    activation: Option<ActivationCriteria>,
    #[serde(default)]
    code_snippets: Option<Vec<CodeSnippet>>,
}

async fn process_skill_repair_output(
    store: &Arc<dyn Store>,
    mission: &Mission,
    response: &str,
) -> Result<(), EngineError> {
    let json_val = match extract_json_from_response(response) {
        Some(v) => v,
        None => {
            debug!("skill-repair: no structured JSON in response");
            return Ok(());
        }
    };
    let repair: SkillRepairMissionOutput =
        serde_json::from_value(json_val).map_err(|e| EngineError::Skill {
            reason: format!("invalid skill-repair output: {e}"),
        })?;

    let Some(triggered_skill) = triggered_skill_provenance(mission, repair.doc_id) else {
        return Err(EngineError::Skill {
            reason: if has_skill_trigger_payload(mission) {
                format!(
                    "skill-repair attempted to modify untriggered skill {}",
                    repair.doc_id.0
                )
            } else {
                "skill-repair requires an active skill trigger payload".into()
            },
        });
    };
    if repair.updated_content.trim().is_empty() {
        return Err(EngineError::Skill {
            reason: format!(
                "skill-repair produced empty updated_content for skill {}",
                repair.doc_id.0
            ),
        });
    }

    let existing =
        store
            .load_memory_doc(repair.doc_id)
            .await?
            .ok_or_else(|| EngineError::Skill {
                reason: format!("skill doc not found: {}", repair.doc_id.0),
            })?;
    if existing.project_id != mission.project_id {
        return Err(EngineError::Skill {
            reason: format!(
                "skill-repair attempted to modify skill {} outside mission project",
                repair.doc_id.0
            ),
        });
    }
    if !existing.is_owned_by(&mission.user_id) {
        return Err(EngineError::AccessDenied {
            user_id: mission.user_id.clone(),
            entity: format!("skill {}", repair.doc_id.0),
        });
    }
    if existing.doc_type != DocType::Skill {
        return Err(EngineError::Skill {
            reason: format!(
                "skill-repair attempted to modify non-skill doc {} ({:?})",
                repair.doc_id.0, existing.doc_type
            ),
        });
    }
    serde_json::from_value::<V2SkillMetadata>(existing.metadata.clone()).map_err(|e| {
        EngineError::Skill {
            reason: format!("invalid skill metadata for {}: {e}", repair.doc_id.0),
        }
    })?;
    let from_version = triggered_skill.version;
    let source_thread_id = mission
        .last_trigger_payload
        .as_ref()
        .and_then(|payload| payload.get("source_thread_id"))
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let summary = if repair.summary.trim().is_empty() {
        format!("Applied {:?} repair", repair.repair_type)
    } else {
        repair.summary.clone()
    };

    let tracker = SkillTracker::new(Arc::clone(store));
    tracker
        .update_skill(
            repair.doc_id,
            repair.updated_content,
            Some(triggered_skill.version),
            move |meta| {
                if let Some(description) = repair.description {
                    meta.description = description;
                }
                if let Some(activation) = repair.activation {
                    meta.activation = activation;
                }
                if let Some(code_snippets) = repair.code_snippets {
                    meta.code_snippets = code_snippets;
                }
                meta.repairs.push(SkillRepairRecord {
                    source_thread_id,
                    from_version,
                    to_version: meta.version,
                    repair_type: repair.repair_type,
                    summary,
                    repaired_at: Some(chrono::Utc::now()),
                });
                if meta.repairs.len() > 10 {
                    let keep_from = meta.repairs.len() - 10;
                    meta.repairs.drain(0..keep_from);
                }
            },
        )
        .await
}

fn has_skill_trigger_payload(mission: &Mission) -> bool {
    mission
        .last_trigger_payload
        .as_ref()
        .and_then(|payload| payload.get("active_skills"))
        .and_then(|value| value.as_array())
        .is_some_and(|skills| !skills.is_empty())
}

fn triggered_skill_provenance(mission: &Mission, doc_id: DocId) -> Option<ActiveSkillProvenance> {
    mission
        .last_trigger_payload
        .as_ref()
        .and_then(|payload| payload.get("active_skills"))
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<ActiveSkillProvenance>>(value).ok())
        .and_then(|skills| skills.into_iter().find(|skill| skill.doc_id == doc_id))
}

/// Collects error messages and deduplicated observed action names in a single
/// pass over `thread.events`.  Previous implementation used separate passes
/// which is wasteful for threads with large event logs.
fn collect_errors_and_actions(thread: &Thread) -> (Vec<String>, Vec<String>) {
    let mut error_messages = Vec::new();
    let mut actions = Vec::new();
    let mut seen = HashSet::new();

    for event in &thread.events {
        match &event.kind {
            crate::types::event::EventKind::ActionFailed {
                action_name, error, ..
            } => {
                if !is_recoverable_action_failure(error) && error_messages.len() < 10 {
                    error_messages.push(format!("{action_name}: {error}"));
                }
                if seen.insert(action_name.clone()) {
                    actions.push(action_name.clone());
                }
            }
            crate::types::event::EventKind::ActionExecuted { action_name, .. } => {
                if seen.insert(action_name.clone()) {
                    actions.push(action_name.clone());
                }
            }
            _ => {}
        }
    }

    (error_messages, actions)
}

fn learning_terminal_state(
    event_kind: &crate::types::event::EventKind,
) -> Option<crate::types::thread::ThreadState> {
    match event_kind {
        crate::types::event::EventKind::StateChanged {
            to: crate::types::thread::ThreadState::Done,
            ..
        } => Some(crate::types::thread::ThreadState::Done),
        crate::types::event::EventKind::StateChanged {
            to: crate::types::thread::ThreadState::Failed,
            ..
        } => Some(crate::types::thread::ThreadState::Failed),
        _ => None,
    }
}

fn should_count_for_conversation_insights(
    terminal_state: crate::types::thread::ThreadState,
) -> bool {
    terminal_state == crate::types::thread::ThreadState::Done
}

fn has_action_failures(thread: &Thread) -> bool {
    thread.events.iter().any(|event| match &event.kind {
        crate::types::event::EventKind::ActionFailed { error, .. } => {
            !is_recoverable_action_failure(error)
        }
        _ => false,
    })
}

fn is_recoverable_auth_failure_text(text: &str) -> bool {
    text.to_ascii_lowercase()
        .contains("authentication required for credential ")
}

fn is_recoverable_action_failure(error: &str) -> bool {
    is_recoverable_auth_failure_text(error)
}

fn action_params_summary(event: &crate::types::event::ThreadEvent) -> Option<&str> {
    match &event.kind {
        crate::types::event::EventKind::ActionExecuted { params_summary, .. }
        | crate::types::event::EventKind::ActionFailed { params_summary, .. } => {
            params_summary.as_deref()
        }
        _ => None,
    }
}

fn contains_word(haystack: &str, word: &str) -> bool {
    for (start, _) in haystack.match_indices(word) {
        let before_ok = start == 0 || haystack.as_bytes()[start - 1].is_ascii_whitespace();
        let end = start + word.len();
        let after_ok = end == haystack.len() || haystack.as_bytes()[end].is_ascii_whitespace();
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

fn has_shell_verification_action(thread: &Thread) -> bool {
    const PHRASE_PATTERNS: &[&str] = &[
        "cargo test",
        "pytest",
        "npm test",
        "pnpm test",
        "yarn test",
        "go test",
        "git diff",
        "git status",
        "gh pr view",
        "gh issue view",
        "cat ",
        "head ",
        "tail ",
        "grep ",
        "rg ",
        "find ",
        "stat ",
    ];
    const WORD_PATTERNS: &[&str] = &["ls", "diff", "status", "view", "show"];

    thread.events.iter().any(|event| match &event.kind {
        crate::types::event::EventKind::ActionExecuted { action_name, .. }
            if action_name == "shell" =>
        {
            action_params_summary(event)
                .map(|summary| {
                    let lower = summary.to_lowercase();
                    PHRASE_PATTERNS
                        .iter()
                        .any(|pattern| lower.contains(pattern))
                        || WORD_PATTERNS.iter().any(|word| contains_word(&lower, word))
                })
                .unwrap_or(false)
        }
        crate::types::event::EventKind::ActionFailed { action_name, .. }
            if action_name == "shell" =>
        {
            action_params_summary(event)
                .map(|summary| {
                    let lower = summary.to_lowercase();
                    PHRASE_PATTERNS
                        .iter()
                        .any(|pattern| lower.contains(pattern))
                        || WORD_PATTERNS.iter().any(|word| contains_word(&lower, word))
                })
                .unwrap_or(false)
        }
        _ => false,
    })
}

fn has_mutating_shell_or_git_action(thread: &Thread) -> bool {
    const PHRASE_PATTERNS: &[&str] = &[
        "apply_patch",
        "git commit",
        "git push",
        "git pull",
        "git merge",
        "git rebase",
        "git cherry-pick",
        "git revert",
        "git reset",
        "git checkout",
        "git switch",
        "cargo fmt",
        "rustfmt",
        "npm install",
        "pnpm install",
        "yarn install",
        "mkdir ",
        "rm ",
        "mv ",
        "cp ",
        "touch ",
        "tee ",
        "sed -i",
        "perl -pi",
    ];
    const WORD_PATTERNS: &[&str] = &[
        "write", "create", "delete", "remove", "rename", "patch", "install", "format",
    ];

    thread.events.iter().any(|event| match &event.kind {
        crate::types::event::EventKind::ActionExecuted { action_name, .. }
        | crate::types::event::EventKind::ActionFailed { action_name, .. }
            if action_name == "shell" || action_name == "git" =>
        {
            action_params_summary(event)
                .map(|summary| {
                    let lower = summary.to_lowercase();
                    PHRASE_PATTERNS
                        .iter()
                        .any(|pattern| lower.contains(pattern))
                        || WORD_PATTERNS.iter().any(|word| contains_word(&lower, word))
                })
                .unwrap_or(false)
        }
        _ => false,
    })
}

fn infer_skill_repair_hints(
    thread: &Thread,
    trace: &ExecutionTrace,
    error_messages: &[String],
    observed_actions: &[String],
) -> Vec<SkillRepairType> {
    let mut hints = Vec::new();
    let mut push_hint = |hint| {
        if !hints.contains(&hint) {
            hints.push(hint);
        }
    };

    let lower_signals = error_messages
        .iter()
        .map(|message| message.to_lowercase())
        .chain(
            trace
                .issues
                .iter()
                .filter(|issue| {
                    !(issue.category == "tool_error"
                        && is_recoverable_auth_failure_text(&issue.description))
                })
                .map(|issue| issue.description.to_lowercase()),
        )
        .collect::<Vec<_>>();

    let recoverable_auth_failures = thread
        .events
        .iter()
        .filter_map(|event| {
            if let crate::types::event::EventKind::ActionFailed { error, .. } = &event.kind
                && is_recoverable_auth_failure_text(error)
            {
                Some(error.to_lowercase())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    if lower_signals
        .iter()
        .chain(recoverable_auth_failures.iter())
        .any(|message| {
            ["auth", "login", "token", "credential", "permission denied"]
                .iter()
                .any(|needle| message.contains(needle))
        })
    {
        push_hint(SkillRepairType::MissingPrerequisite);
    }

    if lower_signals.iter().any(|message| {
        [
            "command not found",
            "no such file",
            "not found",
            "could not find",
            "unknown file",
            "unknown path",
        ]
        .iter()
        .any(|needle| message.contains(needle))
    }) {
        push_hint(SkillRepairType::StaleCommandPath);
    }

    let mutating_actions = observed_actions.iter().any(|action| {
        matches!(
            action.as_str(),
            "write_file" | "apply_patch" | "memory_write" | "skill_install" | "skill_remove"
        )
    }) || has_mutating_shell_or_git_action(thread);
    let verification_actions = observed_actions.iter().any(|action| {
        matches!(
            action.as_str(),
            "read_file" | "memory_read" | "memory_search" | "cargo_test" | "pytest"
        )
    }) || has_shell_verification_action(thread);
    if mutating_actions && !verification_actions {
        push_hint(SkillRepairType::MissingVerification);
    }

    if !error_messages.is_empty() && thread.state == crate::types::thread::ThreadState::Done {
        push_hint(SkillRepairType::MissingPitfall);
    }

    hints
}

fn build_skill_gap_payload(
    thread: &Thread,
    trace: &ExecutionTrace,
    active_skills: &[ActiveSkillProvenance],
) -> Option<serde_json::Value> {
    let (error_messages, observed_actions) = collect_errors_and_actions(thread);
    let repair_hints = infer_skill_repair_hints(thread, trace, &error_messages, &observed_actions);
    if repair_hints.is_empty() {
        return None;
    }

    let issues: Vec<serde_json::Value> = trace
        .issues
        .iter()
        .map(|issue| {
            serde_json::json!({
                "severity": format!("{:?}", issue.severity),
                "category": issue.category.clone(),
                "description": issue.description.clone(),
                "step": issue.step,
            })
        })
        .collect();

    Some(serde_json::json!({
        "source_thread_id": thread.id.0.to_string(),
        "goal": thread.goal,
        "active_skills": active_skills,
        "issues": issues,
        "error_messages": error_messages,
        "observed_actions": observed_actions,
        "repair_hints": repair_hints,
    }))
}

fn thread_completed_successfully(thread: &Thread, trace: &ExecutionTrace) -> bool {
    thread.state == crate::types::thread::ThreadState::Done
        && !has_action_failures(thread)
        && trace
            .issues
            .iter()
            .all(|issue| issue.severity != IssueSeverity::Error)
}

/// The goal for the self-improvement mission (autoresearch-style program).
///
/// This is the "program.md" — a concrete, step-by-step prompt that tells the
/// agent exactly what to do. Inspired by karpathy/autoresearch: the entire
/// research org is a markdown file with an explicit loop.
const SELF_IMPROVEMENT_GOAL: &str = include_str!("../../prompts/mission_self_improvement.md");

/// Well-known title for the fix pattern database.
pub const FIX_PATTERN_DB_TITLE: &str = "fix_pattern_database";

/// Well-known tag for the fix pattern database.
pub const FIX_PATTERN_DB_TAG: &str = "fix_patterns";

/// The goal for the skill extraction mission.
const SKILL_EXTRACTION_GOAL: &str = include_str!("../../prompts/mission_skill_extraction.md");

/// The goal for the skill-repair mission.
const SKILL_REPAIR_GOAL: &str = include_str!("../../prompts/mission_skill_repair.md");

/// The goal for the conversation insights mission.
const CONVERSATION_INSIGHTS_GOAL: &str =
    include_str!("../../prompts/mission_conversation_insights.md");

/// The goal for the expected-behavior mission (user feedback loop).
const EXPECTED_BEHAVIOR_GOAL: &str = include_str!("../../prompts/mission_expected_behavior.md");

/// Seed content for the fix pattern database.
const SEED_FIX_PATTERNS: &str = "\
| Trace pattern | Fix strategy | Location pattern |
|---|---|---|
| Tool X not found | Add name alias or prompt hint about correct name | prompt overlay or effect_adapter |
| TypeError: str indices must be integers | Parse JSON before wrapping | Where tool output is converted |
| NameError: name 'X' not defined | Add prompt hint about using state dict | prompt overlay |
| byte index N is not a char boundary | Replace byte slicing with chars().take(N) | Code that slices strings |
| Model calls nonexistent tool | Add prompt rule listing correct tool name | prompt overlay |
| Model ignores tool results | Improve output metadata format | prompt overlay |
| Excessive steps (>5) for simple task | Add prompt rule or fix tool schema | prompt overlay |
| Code error in REPL output | Add prompt hint about correct API usage | prompt overlay |";

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::sync::Mutex;

    use crate::capability::lease::LeaseManager;
    use crate::capability::policy::PolicyEngine;
    use crate::capability::registry::CapabilityRegistry;
    use crate::traits::effect::EffectExecutor;
    use crate::traits::llm::{LlmCallConfig, LlmOutput};
    use crate::traits::store::Store;
    use crate::types::capability::{ActionDef, CapabilityLease};
    use crate::types::error::EngineError;
    use crate::types::event::ThreadEvent;
    use crate::types::memory::{DocId, DocType, MemoryDoc};
    use crate::types::mission::{Mission, MissionCadence, MissionId, MissionStatus};
    use crate::types::project::{Project, ProjectId};
    use crate::types::step::StepId;
    use crate::types::step::{ActionResult, LlmResponse, Step, TokenUsage};
    use crate::types::thread::{ActiveSkillProvenance, Thread, ThreadId, ThreadState, ThreadType};
    use ironclaw_skills::SkillTrust;
    use ironclaw_skills::types::ActivationCriteria;
    use ironclaw_skills::v2::{SkillMetrics, SkillRepairType, V2SkillMetadata, V2SkillSource};

    // ── TestStore — in-memory Store that persists missions ───

    struct TestStore {
        threads: tokio::sync::RwLock<HashMap<ThreadId, Thread>>,
        missions: tokio::sync::RwLock<HashMap<MissionId, Mission>>,
        docs: tokio::sync::RwLock<Vec<MemoryDoc>>,
        /// Optional gate that blocks the next `save_mission` call until
        /// the test releases it. Used by `fire_mission_arms_cooldown_before_save`
        /// to deterministically observe the in-flight save state.
        save_mission_gate: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        /// Notified when `save_mission` enters the gated wait so the test
        /// knows save is in progress (rather than not yet called).
        save_mission_started: tokio::sync::Notify,
    }

    impl TestStore {
        fn new() -> Self {
            Self {
                threads: tokio::sync::RwLock::new(HashMap::new()),
                missions: tokio::sync::RwLock::new(HashMap::new()),
                docs: tokio::sync::RwLock::new(Vec::new()),
                save_mission_gate: tokio::sync::Mutex::new(None),
                save_mission_started: tokio::sync::Notify::new(),
            }
        }

        /// Block the next `save_mission` call. Returns a sender the test
        /// must signal once it's done observing the in-flight state.
        async fn block_next_save_mission(&self) -> tokio::sync::oneshot::Sender<()> {
            let (tx, rx) = tokio::sync::oneshot::channel();
            *self.save_mission_gate.lock().await = Some(rx);
            tx
        }
    }

    fn make_skill_doc(project_id: ProjectId, user_id: &str, name: &str) -> MemoryDoc {
        let meta = V2SkillMetadata {
            name: name.to_string(),
            version: 1,
            description: format!("{name} description"),
            activation: ActivationCriteria::default(),
            source: V2SkillSource::Extracted,
            trust: SkillTrust::Trusted,
            requires: Default::default(),
            code_snippets: vec![],
            metrics: SkillMetrics::default(),
            parent_version: None,
            revisions: vec![],
            repairs: vec![],
            content_hash: "sha256:test".to_string(),
            bundle_path: None,
            source_url: None,
        };

        let mut doc = MemoryDoc::new(
            project_id,
            user_id,
            DocType::Skill,
            format!("skill:{name}"),
            "Original skill content",
        );
        doc.metadata = serde_json::to_value(&meta).expect("serialize test skill metadata");
        doc
    }

    #[async_trait::async_trait]
    impl Store for TestStore {
        // ── Thread (minimal — save/load needed by ThreadManager) ──
        async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError> {
            self.threads.write().await.insert(thread.id, thread.clone());
            Ok(())
        }
        async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError> {
            Ok(self.threads.read().await.get(&id).cloned())
        }
        async fn list_threads(&self, _: ProjectId, _: &str) -> Result<Vec<Thread>, EngineError> {
            Ok(vec![])
        }
        async fn update_thread_state(
            &self,
            _: ThreadId,
            _: ThreadState,
        ) -> Result<(), EngineError> {
            Ok(())
        }

        // ── Step (noop) ──
        async fn save_step(&self, _: &Step) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_steps(&self, _: ThreadId) -> Result<Vec<Step>, EngineError> {
            Ok(vec![])
        }

        // ── Event (noop) ──
        async fn append_events(&self, _: &[ThreadEvent]) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_events(&self, _: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
            Ok(vec![])
        }

        // ── Project (noop) ──
        async fn save_project(&self, _: &Project) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_project(&self, _: ProjectId) -> Result<Option<Project>, EngineError> {
            Ok(None)
        }

        // ── MemoryDoc ──
        async fn save_memory_doc(&self, doc: &MemoryDoc) -> Result<(), EngineError> {
            let mut docs = self.docs.write().await;
            docs.retain(|d| d.id != doc.id);
            docs.push(doc.clone());
            Ok(())
        }
        async fn load_memory_doc(&self, id: DocId) -> Result<Option<MemoryDoc>, EngineError> {
            Ok(self.docs.read().await.iter().find(|d| d.id == id).cloned())
        }
        async fn list_memory_docs(
            &self,
            project_id: ProjectId,
            _user_id: &str,
        ) -> Result<Vec<MemoryDoc>, EngineError> {
            Ok(self
                .docs
                .read()
                .await
                .iter()
                .filter(|d| d.project_id == project_id)
                .cloned()
                .collect())
        }

        // ── Lease (noop) ──
        async fn save_lease(&self, _: &CapabilityLease) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_active_leases(
            &self,
            _: ThreadId,
        ) -> Result<Vec<CapabilityLease>, EngineError> {
            Ok(vec![])
        }
        async fn revoke_lease(
            &self,
            _: crate::types::capability::LeaseId,
            _: &str,
        ) -> Result<(), EngineError> {
            Ok(())
        }

        // ── Mission (fully implemented) ──
        async fn save_mission(&self, mission: &Mission) -> Result<(), EngineError> {
            // Honor the test gate if one is installed. Take the receiver out
            // of the slot so subsequent saves are unblocked by default — the
            // gate is one-shot per `block_next_save_mission` call.
            let gate = self.save_mission_gate.lock().await.take();
            if let Some(rx) = gate {
                self.save_mission_started.notify_one();
                let _ = rx.await;
            }
            self.missions
                .write()
                .await
                .insert(mission.id, mission.clone());
            Ok(())
        }
        async fn load_mission(&self, id: MissionId) -> Result<Option<Mission>, EngineError> {
            Ok(self.missions.read().await.get(&id).cloned())
        }
        async fn list_missions(
            &self,
            project_id: ProjectId,
            user_id: &str,
        ) -> Result<Vec<Mission>, EngineError> {
            Ok(self
                .missions
                .read()
                .await
                .values()
                .filter(|m| m.project_id == project_id && m.user_id == user_id)
                .cloned()
                .collect())
        }
        async fn list_all_missions(
            &self,
            project_id: ProjectId,
        ) -> Result<Vec<Mission>, EngineError> {
            Ok(self
                .missions
                .read()
                .await
                .values()
                .filter(|m| m.project_id == project_id)
                .cloned()
                .collect())
        }
        async fn update_mission_status(
            &self,
            id: MissionId,
            status: MissionStatus,
        ) -> Result<(), EngineError> {
            if let Some(mission) = self.missions.write().await.get_mut(&id) {
                mission.status = status;
            }
            Ok(())
        }
    }

    // ── MockLlm — returns canned text responses ─────────────

    struct MockLlm {
        responses: Mutex<Vec<LlmOutput>>,
    }

    impl MockLlm {
        fn text(msg: &str) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(vec![LlmOutput {
                    response: LlmResponse::Text(msg.into()),
                    usage: TokenUsage::default(),
                }]),
            })
        }
    }

    #[async_trait::async_trait]
    impl crate::traits::llm::LlmBackend for MockLlm {
        async fn complete(
            &self,
            _: &[crate::types::message::ThreadMessage],
            _: &[ActionDef],
            _: &LlmCallConfig,
        ) -> Result<LlmOutput, EngineError> {
            let mut r = self.responses.lock().unwrap();
            if r.is_empty() {
                Ok(LlmOutput {
                    response: LlmResponse::Text("done".into()),
                    usage: TokenUsage::default(),
                })
            } else {
                Ok(r.remove(0))
            }
        }

        fn model_name(&self) -> &str {
            "mock"
        }
    }

    // ── MockEffects — noop effect executor ───────────────────

    struct MockEffects;

    #[async_trait::async_trait]
    impl EffectExecutor for MockEffects {
        async fn execute_action(
            &self,
            _: &str,
            _: serde_json::Value,
            _: &CapabilityLease,
            _: &crate::traits::effect::ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            Ok(ActionResult {
                call_id: String::new(),
                action_name: String::new(),
                output: serde_json::json!({}),
                is_error: false,
                duration: Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _: &[CapabilityLease],
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(vec![])
        }
    }

    // ── Helper to build a MissionManager with its dependencies ──

    fn make_mission_manager(store: Arc<dyn Store>) -> MissionManager {
        let caps = CapabilityRegistry::new();
        let thread_manager = Arc::new(ThreadManager::new(
            MockLlm::text("done"),
            Arc::new(MockEffects),
            Arc::clone(&store),
            Arc::new(caps),
            Arc::new(LeaseManager::new()),
            Arc::new(PolicyEngine::new()),
        ));
        MissionManager::new(store, thread_manager)
    }

    // ── Tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn create_mission_persists() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "test mission",
                "do the thing",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        let mission = mgr.get_mission(id).await.unwrap();
        assert!(mission.is_some());
        let mission = mission.unwrap();
        assert_eq!(mission.name, "test mission");
        assert_eq!(mission.goal, "do the thing");
        assert_eq!(mission.status, MissionStatus::Active);
        assert_eq!(mission.project_id, project_id);
    }

    #[tokio::test]
    async fn pause_and_resume() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "pausable",
                "goal",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        // Pause
        mgr.pause_mission(id, "test-user").await.unwrap();
        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(mission.status, MissionStatus::Paused);

        // Resume
        mgr.resume_mission(id, "test-user").await.unwrap();
        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(mission.status, MissionStatus::Active);
    }

    #[tokio::test]
    async fn resume_mission_rejects_terminal_states() {
        // Regression: resume_mission must not resurrect Completed/Failed
        // missions. Only Paused → Active is permitted.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "alice",
                "terminal-state-test",
                "goal",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        // Active → resume must fail (only Paused is resumable).
        let err = mgr
            .resume_mission(id, "alice")
            .await
            .expect_err("resume_mission must reject Active missions");
        match err {
            EngineError::Store { reason } => assert!(reason.contains("Active")),
            other => panic!("expected Store error, got {other:?}"),
        }

        // Drive the mission into a terminal state via complete_mission and
        // confirm resume is still rejected.
        mgr.complete_mission(id).await.unwrap();
        let err = mgr
            .resume_mission(id, "alice")
            .await
            .expect_err("resume_mission must reject Completed missions");
        match err {
            EngineError::Store { reason } => assert!(reason.contains("Completed")),
            other => panic!("expected Store error, got {other:?}"),
        }

        // Make sure the status didn't drift after the failed resume calls.
        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(mission.status, MissionStatus::Completed);
    }

    #[tokio::test]
    async fn complete_removes_from_active() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "completable",
                "goal",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        mgr.complete_mission(id).await.unwrap();

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(mission.status, MissionStatus::Completed);
        assert!(mission.is_terminal());

        // Verify removed from active list
        let active = mgr.active.read().await;
        assert!(!active.contains(&id));
    }

    #[tokio::test]
    async fn fire_mission_spawns_thread() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "fireable",
                "build something",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        let thread_id = mgr.fire_mission(id, "test-user", None).await.unwrap();
        assert!(
            thread_id.is_some(),
            "fire_mission should return a thread ID"
        );

        let tid = thread_id.unwrap();

        // Give the spawned thread a moment to finish (MockLlm returns immediately)
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Verify the thread was recorded in mission history
        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.thread_history.contains(&tid),
            "thread should be recorded in mission history"
        );
    }

    #[tokio::test]
    async fn fire_terminal_mission_returns_none() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "terminal",
                "goal",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        // Complete the mission so it becomes terminal
        mgr.complete_mission(id).await.unwrap();

        let result = mgr.fire_mission(id, "test-user", None).await.unwrap();
        assert!(
            result.is_none(),
            "firing a terminal mission should return None"
        );
    }

    #[tokio::test]
    async fn tick_fires_due_missions() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Create a cron mission — create_mission now computes next_fire_at
        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "cron mission",
                "periodic goal",
                MissionCadence::Cron {
                    expression: "* * * * *".into(),
                    timezone: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();

        // Verify next_fire_at was populated by create_mission
        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.next_fire_at.is_some(),
            "create_mission should compute next_fire_at for cron cadence"
        );

        // Move next_fire_at to the past so tick() will fire it
        {
            let mut missions = store.missions.write().await;
            if let Some(mission) = missions.get_mut(&id) {
                mission.next_fire_at = Some(chrono::Utc::now() - chrono::Duration::seconds(60));
            }
        }

        let spawned = mgr.tick("test-user").await.unwrap();
        assert_eq!(spawned.len(), 1, "tick should fire exactly one due mission");

        // Give the spawned thread a moment to finish
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Verify the thread was recorded
        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.thread_history.contains(&spawned[0]),
            "spawned thread should be recorded in mission history"
        );
    }

    // ── E2E Mission Flow Tests ──────────────────────────────

    /// Build a MissionManager with a MockLlm that returns specific text.
    fn make_mission_manager_with_response(store: Arc<dyn Store>, response: &str) -> MissionManager {
        let caps = CapabilityRegistry::new();
        let thread_manager = Arc::new(ThreadManager::new(
            MockLlm::text(response),
            Arc::new(MockEffects),
            Arc::clone(&store),
            Arc::new(caps),
            Arc::new(LeaseManager::new()),
            Arc::new(PolicyEngine::new()),
        ));
        MissionManager::new(store, thread_manager)
    }

    #[tokio::test]
    async fn fire_mission_builds_meta_prompt_with_goal() {
        // The MockLlm returns a simple response. We verify the mission
        // creates a thread and records it.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager_with_response(
            Arc::clone(&store) as Arc<dyn Store>,
            "I searched for news. Found 5 articles.\n\nNext focus: Summarize the articles\nGoal achieved: no",
        );
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "Tech News",
                "Deliver daily tech news briefing",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        let thread_id = mgr.fire_mission(id, "test-user", None).await.unwrap();
        assert!(thread_id.is_some());

        // Wait for background outcome processing
        tokio::time::sleep(Duration::from_millis(200)).await;

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(mission.thread_history.len(), 1);
        assert_eq!(mission.status, MissionStatus::Active); // not completed
    }

    #[tokio::test]
    async fn outcome_processing_extracts_next_focus() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager_with_response(
            Arc::clone(&store) as Arc<dyn Store>,
            "Accomplished: Analyzed the codebase\n\nNext focus: Write tests for the auth module\nGoal achieved: no",
        );
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "Test Coverage",
                "Increase test coverage to 80%",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        mgr.fire_mission(id, "test-user", None).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        // next_focus should be extracted from the response
        assert_eq!(
            mission.current_focus.as_deref(),
            Some("Write tests for the auth module"),
            "next_focus should be extracted from FINAL response"
        );
        // approach_history should have one entry
        assert_eq!(mission.approach_history.len(), 1);
        assert!(mission.approach_history[0].contains("Accomplished"));
    }

    #[tokio::test]
    async fn outcome_processing_detects_goal_achieved() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager_with_response(
            Arc::clone(&store) as Arc<dyn Store>,
            "Coverage is now 82%!\n\nNext focus: none\nGoal achieved: yes",
        );
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "Coverage Mission",
                "Get to 80% coverage",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        // Set success criteria
        {
            let mut missions = store.missions.write().await;
            if let Some(m) = missions.get_mut(&id) {
                m.success_criteria = Some("coverage >= 80%".into());
            }
        }

        mgr.fire_mission(id, "test-user", None).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(
            mission.status,
            MissionStatus::Completed,
            "mission should be completed when goal is achieved"
        );
    }

    #[tokio::test]
    async fn mission_evolves_via_direct_outcome_processing() {
        // Test the outcome processing directly without relying on
        // background task timing.
        let store: Arc<dyn Store> = Arc::new(TestStore::new());
        let project_id = ProjectId::new();

        // Create a mission
        let mission = Mission::new(
            project_id,
            "test-user",
            "Coverage",
            "Increase coverage to 80%",
            MissionCadence::Manual,
        );
        let id = mission.id;
        store.save_mission(&mission).await.unwrap();

        // Simulate fire 1 outcome
        let outcome1 = ThreadOutcome::Completed {
            response: Some(
                "Found 3 uncovered modules.\n\nNext focus: Write tests for db module\nGoal achieved: no".into(),
            ),
        };
        process_mission_outcome(&store, id, ThreadId::new(), &outcome1)
            .await
            .unwrap();

        let mission = store.load_mission(id).await.unwrap().unwrap();
        assert_eq!(
            mission.current_focus.as_deref(),
            Some("Write tests for db module")
        );
        assert_eq!(mission.approach_history.len(), 1);
        assert_eq!(mission.status, MissionStatus::Active);

        // Simulate fire 2 outcome
        let outcome2 = ThreadOutcome::Completed {
            response: Some(
                "Added 15 tests for db module.\n\nNext focus: Write tests for tools module\nGoal achieved: no".into(),
            ),
        };
        process_mission_outcome(&store, id, ThreadId::new(), &outcome2)
            .await
            .unwrap();

        let mission = store.load_mission(id).await.unwrap().unwrap();
        assert_eq!(
            mission.current_focus.as_deref(),
            Some("Write tests for tools module"),
            "focus should evolve between outcomes"
        );
        assert_eq!(mission.approach_history.len(), 2);

        // Simulate fire 3 — goal achieved
        let outcome3 = ThreadOutcome::Completed {
            response: Some("Coverage is 82%!\n\nGoal achieved: yes".into()),
        };
        process_mission_outcome(&store, id, ThreadId::new(), &outcome3)
            .await
            .unwrap();

        let mission = store.load_mission(id).await.unwrap().unwrap();
        assert_eq!(
            mission.status,
            MissionStatus::Completed,
            "mission should complete when goal achieved"
        );
        assert_eq!(mission.approach_history.len(), 3);
    }

    #[tokio::test]
    async fn fire_with_trigger_payload() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager_with_response(
            Arc::clone(&store) as Arc<dyn Store>,
            "Processed the webhook event.\n\nNext focus: none\nGoal achieved: no",
        );
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "GitHub Triage",
                "Triage incoming issues",
                MissionCadence::Webhook {
                    path: "github".into(),
                    secret: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();

        let payload = serde_json::json!({
            "action": "opened",
            "issue": {
                "title": "Bug: login fails",
                "number": 42
            }
        });

        let thread_id = mgr
            .fire_mission(id, "test-user", Some(payload.clone()))
            .await
            .unwrap();
        assert!(thread_id.is_some());

        tokio::time::sleep(Duration::from_millis(200)).await;

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(mission.last_trigger_payload, Some(payload));
        assert_eq!(mission.threads_today, 1);
    }

    #[tokio::test]
    async fn fire_on_system_event_matches_cadence() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager_with_response(Arc::clone(&store) as Arc<dyn Store>, "done");
        let project_id = ProjectId::new();

        // Create an OnSystemEvent mission
        mgr.create_mission(
            project_id,
            "test-user",
            "self-improve",
            "improve prompts",
            MissionCadence::OnSystemEvent {
                source: "engine".into(),
                event_type: "thread_completed_with_issues".into(),
                filters: std::collections::HashMap::new(),
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let spawned = mgr
            .fire_on_system_event(
                "engine",
                "thread_completed_with_issues",
                "test-user",
                Some(serde_json::json!({"issues": []})),
            )
            .await
            .unwrap();
        assert_eq!(spawned.len(), 1, "should fire the matching mission");
    }

    #[tokio::test]
    async fn fire_on_system_event_ignores_non_matching() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager_with_response(Arc::clone(&store) as Arc<dyn Store>, "done");
        let project_id = ProjectId::new();

        // Create an OnSystemEvent mission for a different event
        mgr.create_mission(
            project_id,
            "test-user",
            "webhook handler",
            "handle webhooks",
            MissionCadence::OnSystemEvent {
                source: "github".into(),
                event_type: "push".into(),
                filters: std::collections::HashMap::new(),
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let spawned = mgr
            .fire_on_system_event("engine", "thread_completed_with_issues", "test-user", None)
            .await
            .unwrap();
        assert_eq!(spawned.len(), 0, "should not fire non-matching mission");
    }

    #[tokio::test]
    async fn fire_on_system_event_skips_manual_and_cron() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager_with_response(Arc::clone(&store) as Arc<dyn Store>, "done");
        let project_id = ProjectId::new();

        mgr.create_mission(
            project_id,
            "test-user",
            "manual",
            "goal",
            MissionCadence::Manual,
            Vec::new(),
        )
        .await
        .unwrap();
        mgr.create_mission(
            project_id,
            "test-user",
            "cron",
            "goal",
            MissionCadence::Cron {
                expression: "* * * * *".into(),
                timezone: None,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let spawned = mgr
            .fire_on_system_event("engine", "thread_completed_with_issues", "test-user", None)
            .await
            .unwrap();
        assert_eq!(spawned.len(), 0);
    }

    #[tokio::test]
    async fn self_improvement_outcome_saves_prompt_overlay() {
        let store: Arc<dyn Store> = Arc::new(TestStore::new());
        let project_id = ProjectId::new();

        let mut mission = Mission::new(
            project_id,
            "test-user",
            "self-improve",
            "improve prompts",
            MissionCadence::OnSystemEvent {
                source: "engine".into(),
                event_type: "thread_completed_with_issues".into(),
                filters: std::collections::HashMap::new(),
            },
        );
        mission.metadata = serde_json::json!({"self_improvement": true});
        let id = mission.id;
        store.save_mission(&mission).await.unwrap();

        // Enable self-modification for this test so prompt additions are applied
        unsafe { std::env::set_var("ORCHESTRATOR_SELF_MODIFY", "true") };

        let response = r#"{"prompt_additions": ["9. Never call web_fetch — use http() instead."], "fix_patterns": [], "level": 1}"#;
        let outcome = ThreadOutcome::Completed {
            response: Some(response.into()),
        };
        process_mission_outcome(&store, id, ThreadId::new(), &outcome)
            .await
            .unwrap();

        unsafe { std::env::remove_var("ORCHESTRATOR_SELF_MODIFY") };

        // Verify prompt overlay was saved
        let docs = store.list_memory_docs(project_id, "system").await.unwrap();
        let overlay = docs
            .iter()
            .find(|d| d.title == crate::executor::prompt::PREAMBLE_OVERLAY_TITLE);
        assert!(overlay.is_some(), "prompt overlay should be saved");
        assert!(overlay.unwrap().content.contains("Never call web_fetch"));
    }

    #[tokio::test]
    async fn self_improvement_outcome_saves_fix_patterns() {
        let store: Arc<dyn Store> = Arc::new(TestStore::new());
        let project_id = ProjectId::new();

        let mut mission = Mission::new(
            project_id,
            "test-user",
            "self-improve",
            "improve prompts",
            MissionCadence::Manual,
        );
        mission.metadata = serde_json::json!({"self_improvement": true});
        let id = mission.id;
        store.save_mission(&mission).await.unwrap();

        let response = r#"{"prompt_additions": [], "fix_patterns": [{"pattern": "Tool xyz not found", "strategy": "Add alias xyz -> x-y-z", "location": "effect_adapter"}]}"#;
        let outcome = ThreadOutcome::Completed {
            response: Some(response.into()),
        };
        process_mission_outcome(&store, id, ThreadId::new(), &outcome)
            .await
            .unwrap();

        let docs = store.list_memory_docs(project_id, "system").await.unwrap();
        let patterns = docs.iter().find(|d| d.title == FIX_PATTERN_DB_TITLE);
        assert!(patterns.is_some(), "fix patterns should be saved");
        assert!(patterns.unwrap().content.contains("Tool xyz not found"));
        // Should also contain seed patterns
        assert!(patterns.unwrap().content.contains("NameError"));
    }

    #[tokio::test]
    async fn non_self_improvement_mission_skips_structured_output() {
        let store: Arc<dyn Store> = Arc::new(TestStore::new());
        let project_id = ProjectId::new();

        let mission = Mission::new(
            project_id,
            "test-user",
            "regular",
            "do stuff",
            MissionCadence::Manual,
        );
        let id = mission.id;
        store.save_mission(&mission).await.unwrap();

        // Even if the response has JSON, it should not create overlays
        let response = r#"{"prompt_additions": ["should not appear"], "level": 1}"#;
        let outcome = ThreadOutcome::Completed {
            response: Some(response.into()),
        };
        process_mission_outcome(&store, id, ThreadId::new(), &outcome)
            .await
            .unwrap();

        let docs = store.list_memory_docs(project_id, "system").await.unwrap();
        assert!(docs.is_empty(), "non-SI mission should not create overlay");
    }

    #[tokio::test]
    async fn ensure_self_improvement_mission_creates_on_first_call() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .ensure_self_improvement_mission(project_id, "test-user")
            .await
            .unwrap();

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(mission.name, "self-improvement");
        assert!(is_self_improvement_mission(&mission));
        assert!(matches!(
            mission.cadence,
            MissionCadence::OnSystemEvent { .. }
        ));
        assert_eq!(mission.max_threads_per_day, 5);
        assert_eq!(mission.user_id, "test-user");

        // Fix pattern database should be seeded
        let docs = store.list_memory_docs(project_id, "system").await.unwrap();
        let patterns = docs.iter().find(|d| d.title == FIX_PATTERN_DB_TITLE);
        assert!(patterns.is_some(), "fix patterns should be seeded");
        assert!(patterns.unwrap().content.contains("NameError"));
    }

    #[tokio::test]
    async fn ensure_self_improvement_mission_idempotent() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id1 = mgr
            .ensure_self_improvement_mission(project_id, "test-user")
            .await
            .unwrap();
        let id2 = mgr
            .ensure_self_improvement_mission(project_id, "test-user")
            .await
            .unwrap();

        assert_eq!(id1, id2, "should return the same mission ID");

        // Should only have one mission
        let missions = store.list_missions(project_id, "test-user").await.unwrap();
        assert_eq!(missions.len(), 1);
    }

    #[tokio::test]
    async fn daily_budget_enforced() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager_with_response(Arc::clone(&store) as Arc<dyn Store>, "done");
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "budget test",
                "goal",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        // Set max_threads_per_day to 1
        {
            let mut missions = store.missions.write().await;
            if let Some(m) = missions.get_mut(&id) {
                m.max_threads_per_day = 1;
            }
        }

        // First fire — should work
        let t1 = mgr.fire_mission(id, "test-user", None).await.unwrap();
        assert!(t1.is_some());

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Second fire — should be blocked by budget
        let t2 = mgr.fire_mission(id, "test-user", None).await.unwrap();
        assert!(
            t2.is_none(),
            "second fire should be blocked by daily budget"
        );
    }

    // ── Multi-tenancy tests ────────────────────────────────────

    #[tokio::test]
    async fn per_user_learning_missions_are_isolated() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Bootstrap learning missions for two different users
        mgr.ensure_learning_missions(project_id, "alice")
            .await
            .unwrap();
        mgr.ensure_learning_missions(project_id, "bob")
            .await
            .unwrap();

        // Each user should see only their own missions
        let alice_missions = store.list_missions(project_id, "alice").await.unwrap();
        let bob_missions = store.list_missions(project_id, "bob").await.unwrap();

        assert_eq!(alice_missions.len(), bob_missions.len());
        assert!(
            alice_missions.len() >= 3,
            "at least 3 learning missions per user"
        );

        // No overlap in mission IDs
        let alice_ids: std::collections::HashSet<_> = alice_missions.iter().map(|m| m.id).collect();
        let bob_ids: std::collections::HashSet<_> = bob_missions.iter().map(|m| m.id).collect();
        assert!(
            alice_ids.is_disjoint(&bob_ids),
            "alice and bob should have separate mission instances"
        );

        // Verify user_id is set correctly on all missions
        assert!(alice_missions.iter().all(|m| m.user_id == "alice"));
        assert!(bob_missions.iter().all(|m| m.user_id == "bob"));
    }

    #[tokio::test]
    async fn pause_resume_does_not_cross_users() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Create a mission for alice
        let alice_id = mgr
            .create_mission(
                project_id,
                "alice",
                "alice-task",
                "goal",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        // Create a mission for bob
        let bob_id = mgr
            .create_mission(
                project_id,
                "bob",
                "bob-task",
                "goal",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        // Alice pauses her own mission — should succeed
        mgr.pause_mission(alice_id, "alice").await.unwrap();
        let alice_mission = mgr.get_mission(alice_id).await.unwrap().unwrap();
        assert_eq!(alice_mission.status, MissionStatus::Paused);

        // Bob's mission should be unaffected
        let bob_mission = mgr.get_mission(bob_id).await.unwrap().unwrap();
        assert_eq!(bob_mission.status, MissionStatus::Active);

        // Bob tries to resume alice's mission — should fail
        let result = mgr.resume_mission(alice_id, "bob").await;
        assert!(
            result.is_err(),
            "bob should not be able to resume alice's mission"
        );
        assert!(
            matches!(result.unwrap_err(), EngineError::AccessDenied { .. }),
            "should be AccessDenied"
        );

        // Alice resumes her own mission — should succeed
        mgr.resume_mission(alice_id, "alice").await.unwrap();
        let alice_mission = mgr.get_mission(alice_id).await.unwrap().unwrap();
        assert_eq!(alice_mission.status, MissionStatus::Active);
    }

    #[tokio::test]
    async fn user_cannot_pause_another_users_learning_mission() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Bootstrap per-user learning missions
        mgr.ensure_learning_missions(project_id, "alice")
            .await
            .unwrap();
        mgr.ensure_learning_missions(project_id, "bob")
            .await
            .unwrap();

        // Get alice's self-improvement mission
        let alice_missions = store.list_missions(project_id, "alice").await.unwrap();
        let alice_self_imp = alice_missions
            .iter()
            .find(|m| is_self_improvement_mission(m))
            .expect("alice should have a self-improvement mission");

        // Bob tries to pause alice's self-improvement — should fail
        let result = mgr.pause_mission(alice_self_imp.id, "bob").await;
        assert!(
            matches!(result.unwrap_err(), EngineError::AccessDenied { .. }),
            "bob cannot pause alice's learning mission"
        );

        // Alice pauses her own — should succeed
        mgr.pause_mission(alice_self_imp.id, "alice").await.unwrap();
        let m = mgr.get_mission(alice_self_imp.id).await.unwrap().unwrap();
        assert_eq!(m.status, MissionStatus::Paused);

        // Bob's self-improvement should still be active
        let bob_missions = store.list_missions(project_id, "bob").await.unwrap();
        let bob_self_imp = bob_missions
            .iter()
            .find(|m| is_self_improvement_mission(m))
            .unwrap();
        assert_eq!(bob_self_imp.status, MissionStatus::Active);
    }

    #[tokio::test]
    async fn system_mission_visible_to_all_via_with_shared() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Create a system mission (admin-installed shared mission)
        let system_id = mgr
            .create_mission(
                project_id,
                "system",
                "shared-monitoring",
                "monitor uptime",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        // Create a user mission
        let _user_id = mgr
            .create_mission(
                project_id,
                "alice",
                "alice-task",
                "do stuff",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        // Alice's list_missions (strict) only shows her own
        let alice_strict = store.list_missions(project_id, "alice").await.unwrap();
        assert_eq!(alice_strict.len(), 1);

        // list_missions_with_shared shows both alice's and system's
        let alice_shared = store
            .list_missions_with_shared(project_id, "alice")
            .await
            .unwrap();
        assert_eq!(alice_shared.len(), 2);
        assert!(alice_shared.iter().any(|m| m.id == system_id));

        // Bob sees only the system mission (no personal missions)
        let bob_shared = store
            .list_missions_with_shared(project_id, "bob")
            .await
            .unwrap();
        assert_eq!(bob_shared.len(), 1);
        assert_eq!(bob_shared[0].id, system_id);
    }

    #[tokio::test]
    async fn shared_mission_management_is_open_at_engine_layer() {
        // Contract pinned by this test (matches the doc-comment on
        // `resume_mission` and the ownership tightening in PR #2126/#2130):
        //
        //     "Shared missions can only be managed by shared owners
        //      (system user)."
        //
        // i.e. shared (system-owned) missions are NOT manageable by regular
        // users at the engine layer. The web handler used to be expected to
        // gate admin-role; the engine now enforces shared-owner identity
        // directly so the contract holds even when the engine is called
        // outside the web handler.
        //
        // The user-vs-user case for non-shared missions is covered by
        // `pause_resume_does_not_cross_users`.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // "system" maps to OwnerId::Shared via LEGACY_SHARED_OWNER_ID.
        let system_id = mgr
            .create_mission(
                project_id,
                "system",
                "shared-mission",
                "shared goal",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();
        let mission = mgr.get_mission(system_id).await.unwrap().unwrap();
        assert!(
            mission.owner_id().is_shared(),
            "missions owned by 'system' must be classified as shared"
        );

        // Regular users CANNOT pause a shared mission — engine returns
        // AccessDenied.
        let alice_pause = mgr.pause_mission(system_id, "alice").await;
        assert!(
            matches!(alice_pause, Err(EngineError::AccessDenied { .. })),
            "regular users must not pause shared missions; got {:?}",
            alice_pause
        );

        // The system user (canonical shared-owner identity) can manage it.
        mgr.pause_mission(system_id, "system").await.unwrap();
        let m = mgr.get_mission(system_id).await.unwrap().unwrap();
        assert_eq!(m.status, MissionStatus::Paused);

        // Regular users also cannot resume.
        let bob_resume = mgr.resume_mission(system_id, "bob").await;
        assert!(
            matches!(bob_resume, Err(EngineError::AccessDenied { .. })),
            "regular users must not resume shared missions; got {:?}",
            bob_resume
        );

        // System user resume works.
        mgr.resume_mission(system_id, "system").await.unwrap();
        let m = mgr.get_mission(system_id).await.unwrap().unwrap();
        assert_eq!(m.status, MissionStatus::Active);
    }

    #[tokio::test]
    async fn fire_mission_ownership_check() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Create alice's mission
        let alice_id = mgr
            .create_mission(
                project_id,
                "alice",
                "alice-only",
                "private goal",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        // Bob cannot fire alice's mission
        let result = mgr.fire_mission(alice_id, "bob", None).await;
        assert!(
            matches!(result.unwrap_err(), EngineError::AccessDenied { .. }),
            "bob cannot fire alice's mission"
        );

        // Alice can fire her own
        let tid = mgr.fire_mission(alice_id, "alice", None).await.unwrap();
        assert!(tid.is_some());
    }

    #[tokio::test]
    async fn fire_on_system_event_scoped_to_user() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Bootstrap per-user learning missions
        mgr.ensure_learning_missions(project_id, "alice")
            .await
            .unwrap();
        mgr.ensure_learning_missions(project_id, "bob")
            .await
            .unwrap();

        // Count active missions for each user
        let alice_missions = store.list_missions(project_id, "alice").await.unwrap();
        let bob_missions = store.list_missions(project_id, "bob").await.unwrap();
        let alice_self_imp = alice_missions
            .iter()
            .find(|m| is_self_improvement_mission(m))
            .unwrap();
        let bob_self_imp = bob_missions
            .iter()
            .find(|m| is_self_improvement_mission(m))
            .unwrap();

        // Pause bob's self-improvement
        mgr.pause_mission(bob_self_imp.id, "bob").await.unwrap();

        // Fire system event as alice — should fire alice's missions, not bob's
        let payload = serde_json::json!({"source_thread_id": "test", "goal": "test"});
        let spawned = mgr
            .fire_on_system_event(
                "engine",
                "thread_completed_with_issues",
                "alice",
                Some(payload),
            )
            .await
            .unwrap();

        // Should have fired alice's self-improvement (active) but not bob's (paused)
        assert!(!spawned.is_empty(), "alice's self-improvement should fire");

        // Verify spawned thread belongs to alice
        tokio::time::sleep(Duration::from_millis(50)).await;
        for tid in &spawned {
            if let Some(thread) = store.load_thread(*tid).await.unwrap() {
                assert_eq!(
                    thread.user_id, "alice",
                    "spawned thread should belong to alice"
                );
            }
        }

        // Verify bob's self-improvement is still paused and was not fired
        let bob_m = mgr.get_mission(bob_self_imp.id).await.unwrap().unwrap();
        assert_eq!(bob_m.status, MissionStatus::Paused);
        assert!(
            bob_m.thread_history.is_empty(),
            "bob's paused mission should not have spawned threads"
        );

        // Alice's should have recorded the thread
        let alice_m = mgr.get_mission(alice_self_imp.id).await.unwrap().unwrap();
        assert!(
            !alice_m.thread_history.is_empty(),
            "alice's mission should have recorded the spawned thread"
        );
    }

    /// Helper: create an event mission with the reactive-default guardrails
    /// disabled so the test can fire it repeatedly without tripping cooldown
    /// or daily caps. Patterns are caller-supplied; everything else stays
    /// at the engine defaults *except* the guardrails we explicitly null out.
    async fn create_unguarded_event_mission(
        mgr: &MissionManager,
        project_id: ProjectId,
        user_id: &str,
        name: &str,
        pattern: &str,
        channel: Option<&str>,
    ) -> MissionId {
        let id = mgr
            .create_mission(
                project_id,
                user_id,
                name,
                "react to events",
                MissionCadence::OnEvent {
                    event_pattern: pattern.to_string(),
                    channel: channel.map(String::from),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        // Disable reactive defaults for tests that want to assert the
        // matcher behavior without tripping cooldown / max_concurrent.
        mgr.update_mission(
            id,
            user_id,
            MissionUpdate {
                cooldown_secs: Some(0),
                max_concurrent: Some(0),
                max_threads_per_day: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn fire_on_message_event_matches_pattern_and_channel_filter() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Mission with a channel-scoped message event trigger.
        let id = create_unguarded_event_mission(
            &mgr,
            project_id,
            "alice",
            "PR review nudge",
            "review requested",
            Some("github"),
        )
        .await;

        // Wrong channel — should NOT fire even though pattern matches.
        let spawned = mgr
            .fire_on_message_event("slack", "review requested on PR #42", "alice", None)
            .await
            .unwrap();
        assert!(spawned.is_empty(), "wrong channel should not fire");

        // Right channel, wrong pattern — should NOT fire.
        let spawned = mgr
            .fire_on_message_event("github", "build green", "alice", None)
            .await
            .unwrap();
        assert!(spawned.is_empty(), "wrong pattern should not fire");

        // Right channel, right pattern — SHOULD fire.
        let spawned = mgr
            .fire_on_message_event(
                "github",
                "review requested on PR #42",
                "alice",
                Some(serde_json::json!({"pr": 42})),
            )
            .await
            .unwrap();
        assert_eq!(
            spawned.len(),
            1,
            "matching event should fire exactly one mission"
        );

        // Channel filter is case-insensitive.
        let spawned = mgr
            .fire_on_message_event("GitHub", "review requested again", "alice", None)
            .await
            .unwrap();
        assert_eq!(spawned.len(), 1, "channel match should be case-insensitive");

        // Mission's thread history should now reflect both fires.
        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(mission.thread_history.len(), 2);
    }

    #[tokio::test]
    async fn fire_on_message_event_without_channel_filter_matches_any_channel() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Mission with no channel filter — should match any channel.
        create_unguarded_event_mission(
            &mgr,
            project_id,
            "alice",
            "Universal pattern",
            "deploy now",
            None,
        )
        .await;

        for channel in &["github", "slack", "gateway", "repl"] {
            let spawned = mgr
                .fire_on_message_event(channel, "please deploy now thanks", "alice", None)
                .await
                .unwrap();
            assert_eq!(
                spawned.len(),
                1,
                "no channel filter should match channel {channel}"
            );
        }
    }

    #[tokio::test]
    async fn fire_on_message_event_respects_owner_scope() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Alice owns a mission.
        create_unguarded_event_mission(&mgr, project_id, "alice", "Alice mission", "ping", None)
            .await;

        // Bob fires the event with a matching pattern — should NOT fire
        // alice's mission (per-user scoping).
        let spawned = mgr
            .fire_on_message_event("gateway", "ping", "bob", None)
            .await
            .unwrap();
        assert!(
            spawned.is_empty(),
            "events from other users must not fire missions they don't own"
        );

        // Alice fires the event — SHOULD fire her mission.
        let spawned = mgr
            .fire_on_message_event("gateway", "ping", "alice", None)
            .await
            .unwrap();
        assert_eq!(spawned.len(), 1);
    }

    #[tokio::test]
    async fn fire_on_webhook_matches_path() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        mgr.create_mission(
            project_id,
            "alice",
            "GitHub webhook",
            "Handle GitHub events",
            MissionCadence::Webhook {
                path: "github".into(),
                secret: None,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        // Wrong path — should NOT fire.
        let spawned = mgr.fire_on_webhook("slack", "alice", None).await.unwrap();
        assert!(spawned.is_empty());

        // Right path — SHOULD fire.
        let spawned = mgr
            .fire_on_webhook(
                "github",
                "alice",
                Some(serde_json::json!({"action": "opened"})),
            )
            .await
            .unwrap();
        assert_eq!(spawned.len(), 1);
    }

    /// Regression for the substring-match flooding bug:
    /// `text.contains("review requested")` would match unrelated phrases
    /// like "I just reviewed your request" — way too loose. The matcher
    /// is now regex-based, so word-boundary-aware patterns no longer
    /// flood on accidental substrings.
    #[tokio::test]
    async fn fire_on_message_event_uses_regex_with_word_boundaries() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Word-boundary regex for "deploy".
        create_unguarded_event_mission(
            &mgr,
            project_id,
            "alice",
            "Deploy watcher",
            r"\bdeploy\b",
            None,
        )
        .await;

        // Should NOT match: "deployed" / "deployment" / "redeploy".
        for noisy in &[
            "I just deployed the change",
            "the deployment finished",
            "going to redeploy later",
        ] {
            let spawned = mgr
                .fire_on_message_event("gateway", noisy, "alice", None)
                .await
                .unwrap();
            assert!(spawned.is_empty(), "regex with \\b must not match: {noisy}");
        }

        // SHOULD match: standalone "deploy".
        let spawned = mgr
            .fire_on_message_event("gateway", "please deploy now", "alice", None)
            .await
            .unwrap();
        assert_eq!(spawned.len(), 1, "standalone 'deploy' must match");
    }

    /// Regression: an OnEvent mission created via `create_mission` without
    /// explicit guardrails must inherit reactive defaults (cooldown 300s,
    /// max_concurrent 1, daily cap 24) so accidentally-loose patterns
    /// can't burn the LLM budget.
    #[tokio::test]
    async fn event_triggered_missions_get_reactive_defaults() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "alice",
                "Default reactive mission",
                "react",
                MissionCadence::OnEvent {
                    event_pattern: "anything".into(),
                    channel: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(
            mission.cooldown_secs, 300,
            "OnEvent missions default to a 5-minute cooldown"
        );
        assert_eq!(
            mission.max_concurrent, 1,
            "OnEvent missions default to single-instance"
        );
        assert_eq!(
            mission.max_threads_per_day, 24,
            "OnEvent missions default to 24 fires/day"
        );
    }

    /// Manual / Cron missions retain the prior generous defaults — they
    /// are self-paced and don't risk flooding from external events.
    #[tokio::test]
    async fn manual_and_cron_missions_keep_proactive_defaults() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let manual_id = mgr
            .create_mission(
                project_id,
                "alice",
                "manual",
                "do it on demand",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();
        let manual = mgr.get_mission(manual_id).await.unwrap().unwrap();
        assert_eq!(manual.cooldown_secs, 0);
        assert_eq!(manual.max_concurrent, 0);
        assert_eq!(manual.max_threads_per_day, 10);

        let cron_id = mgr
            .create_mission(
                project_id,
                "alice",
                "cron",
                "every six hours",
                MissionCadence::Cron {
                    expression: "0 */6 * * *".into(),
                    timezone: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let cron = mgr.get_mission(cron_id).await.unwrap().unwrap();
        assert_eq!(cron.cooldown_secs, 0);
        assert_eq!(cron.max_concurrent, 0);
        assert_eq!(cron.max_threads_per_day, 10);
    }

    /// The per-user sliding-window rate limiter must refuse fires once
    /// the cap is reached and recover after the window slides past.
    #[tokio::test]
    async fn per_user_rate_limit_blocks_excess_fires() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>).with_rate_limit(
            FireRateLimit {
                max_fires: 3,
                window: std::time::Duration::from_secs(60),
            },
        );
        let project_id = ProjectId::new();

        create_unguarded_event_mission(
            &mgr,
            project_id,
            "alice",
            "rate-limited mission",
            r"go",
            None,
        )
        .await;

        // First 3 fires should succeed; the 4th should be silently dropped.
        for i in 0..3 {
            let spawned = mgr
                .fire_on_message_event("gateway", "go", "alice", None)
                .await
                .unwrap();
            assert_eq!(spawned.len(), 1, "fire {i} should succeed");
        }
        let spawned = mgr
            .fire_on_message_event("gateway", "go", "alice", None)
            .await
            .unwrap();
        assert!(spawned.is_empty(), "rate-limited fire should be dropped");
    }

    /// `BudgetGate::allow_mission_fire` returning false must abort the
    /// fire without spawning a thread or recording history.
    #[tokio::test]
    async fn budget_gate_can_refuse_mission_fires() {
        struct DenyAll;
        #[async_trait::async_trait]
        impl BudgetGate for DenyAll {
            async fn allow_mission_fire(&self, _user_id: &str, _mission_id: MissionId) -> bool {
                false
            }
        }

        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>)
            .with_budget_gate(Arc::new(DenyAll));
        let project_id = ProjectId::new();

        let id =
            create_unguarded_event_mission(&mgr, project_id, "alice", "blocked", r"go", None).await;

        let spawned = mgr
            .fire_on_message_event("gateway", "go", "alice", None)
            .await
            .unwrap();
        assert!(spawned.is_empty(), "BudgetGate denial must block the fire");

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.thread_history.is_empty(),
            "denied fire must not record any threads"
        );
    }

    /// Updating a mission must evict its cached compiled regex so the next
    /// match attempt picks up the new pattern.
    #[tokio::test]
    async fn updating_event_pattern_invalidates_regex_cache() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id =
            create_unguarded_event_mission(&mgr, project_id, "alice", "swappable", r"alpha", None)
                .await;

        // Initial pattern matches "alpha".
        let spawned = mgr
            .fire_on_message_event("gateway", "alpha", "alice", None)
            .await
            .unwrap();
        assert_eq!(spawned.len(), 1);

        // Swap the cadence to a new pattern.
        mgr.update_mission(
            id,
            "alice",
            MissionUpdate {
                cadence: Some(MissionCadence::OnEvent {
                    event_pattern: r"beta".into(),
                    channel: None,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // The old pattern must no longer match.
        let spawned = mgr
            .fire_on_message_event("gateway", "alpha", "alice", None)
            .await
            .unwrap();
        assert!(spawned.is_empty(), "stale regex cache must be evicted");

        // The new pattern must match.
        let spawned = mgr
            .fire_on_message_event("gateway", "beta", "alice", None)
            .await
            .unwrap();
        assert_eq!(spawned.len(), 1, "new pattern must take effect");
    }

    // ── routine-fix-history regression tests ─────────────────────────
    //
    // Tests in this section pin invariants whose v1 routine analogs were
    // historically broken (or whose fix went into a v1 routine code path
    // that has no v2 equivalent — we add them here to make sure missions
    // never regress the same bug).

    /// Mirrors v1 routine fix #1372 / #1374: a fired mission with
    /// `max_concurrent = N` and N already-running threads must refuse to
    /// fire again. The check is in `fire_mission` after the cooldown gate.
    /// This test pins it through the public surface so a future refactor
    /// can't drop the check without failing here.
    #[tokio::test]
    async fn fire_mission_blocks_when_max_concurrent_reached() {
        use crate::types::thread::{Thread, ThreadConfig, ThreadType};
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "alice",
                "single instance",
                "do exactly one thing at a time",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();
        // Set max_concurrent=1 explicitly (Manual missions default to 0).
        mgr.update_mission(
            id,
            "alice",
            MissionUpdate {
                max_concurrent: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Pre-seed a Running thread for this mission so the next fire
        // sees max_concurrent already saturated. ThreadType::Mission with
        // the default `Created` state is non-terminal in
        // count_running_threads (which only treats Done/Failed as
        // terminal).
        let thread = Thread::new(
            "preseeded",
            ThreadType::Mission,
            project_id,
            "alice",
            ThreadConfig::default(),
        );
        let preseeded_id = thread.id;
        store.save_thread(&thread).await.unwrap();
        let mut mission = mgr.get_mission(id).await.unwrap().unwrap();
        mission.thread_history.push(preseeded_id);
        store.save_mission(&mission).await.unwrap();

        // Fire — should be refused with Ok(None), not an error.
        let outcome = mgr.fire_mission(id, "alice", None).await.unwrap();
        assert!(
            outcome.is_none(),
            "max_concurrent=1 with one running thread must block the next fire"
        );

        // The mission's thread_history must NOT have grown.
        let after = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(
            after.thread_history.len(),
            1,
            "blocked fire must not record a new thread"
        );
    }

    /// Mirrors v1 routine fix #1321: notification summaries must be
    /// truncated before broadcasting so a runaway response can't OOM
    /// chat-channel adapters or saturate SSE buffers. The full text stays
    /// in `mission.approach_history` untruncated.
    #[test]
    fn truncate_notification_text_caps_long_strings() {
        let huge = "x".repeat(MAX_NOTIFICATION_RESPONSE_BYTES * 3);
        let truncated = truncate_notification_text(&huge);
        assert!(
            truncated.len() <= MAX_NOTIFICATION_RESPONSE_BYTES + 4,
            "truncated text must fit within the cap (plus the ellipsis byte): got {}",
            truncated.len()
        );
        assert!(
            truncated.ends_with('…'),
            "truncation must end with an ellipsis"
        );

        let small = "fits within the cap";
        assert_eq!(
            truncate_notification_text(small),
            small,
            "strings under the cap must pass through unchanged"
        );
    }

    /// Mirrors v1 routine fix's `floor_char_boundary` change: truncation
    /// MUST NOT split a multi-byte UTF-8 sequence. The naive approach
    /// (`&s[..MAX]`) would panic on a multi-byte character that straddles
    /// the byte index.
    #[test]
    fn truncate_notification_text_is_utf8_safe() {
        // Construct a string where a multi-byte char straddles the byte cap.
        // "ñ" is 2 bytes (0xC3 0xB1). We want byte position MAX_BYTES to
        // land in the middle of one.
        let prefix = "a".repeat(MAX_NOTIFICATION_RESPONSE_BYTES - 1);
        let mut input = prefix;
        input.push('ñ'); // 2 bytes — second byte is at MAX_BYTES
        input.push_str(&"b".repeat(100));
        assert!(input.len() > MAX_NOTIFICATION_RESPONSE_BYTES);

        // Must not panic — the bug would slice a multi-byte char in half.
        let truncated = truncate_notification_text(&input);
        // And the result must be valid UTF-8 (it's a String, so by
        // construction it is — but the assertion makes the invariant
        // explicit).
        assert!(truncated.is_char_boundary(truncated.len()));
        // The 'ñ' must NOT have been split: either it's in the result
        // wholly, or it was dropped wholly.
        assert!(
            !truncated.ends_with('a'),
            "truncation should have stopped at the multi-byte char boundary, not after it"
        );
    }

    /// Mirrors v1 routine fix #1255: when a mission is deleted (the v2
    /// analog of `routine_delete`), its compiled regex cache entry MUST
    /// be evicted so a future mission with the same id can't accidentally
    /// pick up a stale pattern. This pins the eviction call already in
    /// `complete_mission`.
    #[tokio::test]
    async fn complete_mission_evicts_event_regex_cache() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = create_unguarded_event_mission(
            &mgr,
            project_id,
            "alice",
            "to be deleted",
            r"hello",
            None,
        )
        .await;

        // Force regex compile + cache populate.
        let _ = mgr
            .fire_on_message_event("gateway", "hello", "alice", None)
            .await
            .unwrap();
        assert!(
            mgr.event_regex_cache.read().await.contains_key(&id),
            "regex cache should hold the compiled pattern after first match"
        );

        mgr.complete_mission(id).await.unwrap();
        assert!(
            !mgr.event_regex_cache.read().await.contains_key(&id),
            "complete_mission must evict the cached compiled regex"
        );
    }

    /// Mirrors v1 routine fix #1374: failure-path outcomes must produce a
    /// notification, not silently swallow the error. Without this, a
    /// failed mission run leaves the user with no signal that anything
    /// went wrong.
    #[tokio::test]
    async fn failed_outcome_emits_error_notification() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "alice",
                "may fail",
                "do the risky thing",
                MissionCadence::Manual,
                vec!["gateway".to_string()],
            )
            .await
            .unwrap();

        let mut rx = mgr.subscribe_notifications();

        let synthetic_thread_id = crate::types::thread::ThreadId::new();
        process_mission_outcome_and_notify(
            &(Arc::clone(&store) as Arc<dyn Store>),
            id,
            synthetic_thread_id,
            &ThreadOutcome::Failed {
                error: "container exited 137".into(),
            },
            mgr.notification_tx_for_test(),
            None,
        )
        .await
        .unwrap();

        let notification = rx
            .try_recv()
            .expect("Failed outcome must emit a notification");
        assert!(notification.is_error, "is_error flag must be set");
        assert_eq!(notification.notify_channels, vec!["gateway".to_string()]);
        assert!(
            notification
                .response
                .as_deref()
                .is_some_and(|r| r.contains("container exited 137")),
            "notification response must surface the underlying error message; got {:?}",
            notification.response
        );

        // Same for MaxIterations — historically the silent-fail case.
        process_mission_outcome_and_notify(
            &(Arc::clone(&store) as Arc<dyn Store>),
            id,
            synthetic_thread_id,
            &ThreadOutcome::MaxIterations,
            mgr.notification_tx_for_test(),
            None,
        )
        .await
        .unwrap();

        let notification = rx
            .try_recv()
            .expect("MaxIterations must emit a notification");
        assert!(notification.is_error, "MaxIterations must set is_error");
    }

    #[tokio::test]
    async fn outcome_processor_reconciles_missing_fire_accounting() {
        // Regression: when `fire_mission`'s post-spawn `save_mission` fails,
        // the persisted mission is missing the new thread_id, threads_today
        // bump, last_fire_at stamp, and (for cron) advanced next_fire_at.
        // The outcome processor must reconcile these fields the next time
        // it runs so the durable state catches up — otherwise tick re-fires
        // against the stale schedule once the in-memory cooldown elapses.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Create a cron mission with `next_fire_at` already in the past, to
        // mimic the post-failed-fire state directly. (Going through
        // `fire_mission` with a fault-injecting store would require a new
        // TestStore variant; this is the equivalent end state.)
        let id = mgr
            .create_mission(
                project_id,
                "alice",
                "reconcile-test",
                "g",
                MissionCadence::Cron {
                    expression: "* * * * *".into(),
                    timezone: None,
                },
                vec![],
            )
            .await
            .unwrap();
        {
            let mut missions = store.missions.write().await;
            let mission = missions.get_mut(&id).unwrap();
            mission.next_fire_at = Some(chrono::Utc::now() - chrono::Duration::seconds(120));
            mission.threads_today = 0;
            mission.thread_history.clear();
            mission.last_fire_at = None;
        }

        // Run the outcome processor with a thread_id that the persisted
        // mission has never seen — exactly the state a failed `save_mission`
        // would leave us in.
        let orphan_thread_id = crate::types::thread::ThreadId::new();
        // Pass an explicit `original_fire_at` so the test exercises the
        // production-equivalent path where fire_mission's instant flows
        // through the watcher into reconcile (instead of falling back to
        // `now`).
        let original_fire_at = chrono::Utc::now() - chrono::Duration::seconds(30);
        process_mission_outcome_and_notify(
            &(Arc::clone(&store) as Arc<dyn Store>),
            id,
            orphan_thread_id,
            &ThreadOutcome::Completed {
                response: Some("done".into()),
            },
            mgr.notification_tx_for_test(),
            Some(original_fire_at),
        )
        .await
        .unwrap();

        let reloaded = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            reloaded.thread_history.contains(&orphan_thread_id),
            "outcome processor must idempotently append the missing thread_id"
        );
        assert_eq!(
            reloaded.threads_today, 1,
            "threads_today must catch up after reconcile"
        );
        assert_eq!(
            reloaded.last_fire_at,
            Some(original_fire_at),
            "last_fire_at must be reconciled to the original fire instant, not `now`"
        );
        assert!(
            reloaded
                .next_fire_at
                .is_some_and(|next| next > chrono::Utc::now()),
            "next_fire_at must be advanced past now() after reconcile, got {:?}",
            reloaded.next_fire_at
        );

        // Reconcile is idempotent: replaying with the same thread_id must
        // not double-count threads_today or duplicate the history entry.
        process_mission_outcome_and_notify(
            &(Arc::clone(&store) as Arc<dyn Store>),
            id,
            orphan_thread_id,
            &ThreadOutcome::Completed {
                response: Some("done".into()),
            },
            mgr.notification_tx_for_test(),
            Some(original_fire_at),
        )
        .await
        .unwrap();
        let reloaded = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(
            reloaded
                .thread_history
                .iter()
                .filter(|t| **t == orphan_thread_id)
                .count(),
            1,
            "thread_history must not duplicate on replay"
        );
        assert_eq!(
            reloaded.threads_today, 1,
            "threads_today must not double-count on replay"
        );
    }

    /// A pattern that fails to compile (or exceeds the size cap) must be
    /// logged and never match — it must not panic, hang, or fall through
    /// to a substring search.
    #[tokio::test]
    async fn invalid_event_regex_never_matches() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // `[` is not a valid regex; compilation must fail.
        create_unguarded_event_mission(&mgr, project_id, "alice", "broken pattern", "[", None)
            .await;

        let spawned = mgr
            .fire_on_message_event("gateway", "anything", "alice", None)
            .await
            .unwrap();
        assert!(spawned.is_empty(), "invalid regex must not match anything");
    }

    #[tokio::test]
    async fn ensure_learning_missions_idempotent_per_user() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Call twice for the same user
        mgr.ensure_learning_missions(project_id, "alice")
            .await
            .unwrap();
        mgr.ensure_learning_missions(project_id, "alice")
            .await
            .unwrap();

        // Should not create duplicates
        let alice_missions = store.list_missions(project_id, "alice").await.unwrap();
        let self_imp_count = alice_missions
            .iter()
            .filter(|m| is_self_improvement_mission(m))
            .count();
        assert_eq!(
            self_imp_count, 1,
            "should not duplicate self-improvement mission"
        );
    }

    // ── Cron scheduling tests (#1944) ─────────────────────────

    #[tokio::test]
    async fn create_cron_mission_sets_next_fire_at() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "cron test",
                "periodic goal",
                MissionCadence::Cron {
                    expression: "0 */6 * * *".into(),
                    timezone: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.next_fire_at.is_some(),
            "cron mission should have next_fire_at computed on creation"
        );
        assert!(
            mission.next_fire_at.unwrap() > chrono::Utc::now(),
            "next_fire_at should be in the future"
        );
    }

    #[tokio::test]
    async fn create_manual_mission_has_no_next_fire_at() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "manual test",
                "goal",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.next_fire_at.is_none(),
            "manual mission should not have next_fire_at"
        );
    }

    #[tokio::test]
    async fn fire_mission_advances_next_fire_at() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "cron advance",
                "periodic goal",
                MissionCadence::Cron {
                    expression: "* * * * *".into(),
                    timezone: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();

        // Move next_fire_at to the past so tick fires it
        {
            let mut missions = store.missions.write().await;
            if let Some(mission) = missions.get_mut(&id) {
                mission.next_fire_at = Some(chrono::Utc::now() - chrono::Duration::seconds(60));
            }
        }

        let spawned = mgr.tick("test-user").await.unwrap();
        assert_eq!(spawned.len(), 1);

        // After firing, next_fire_at should be advanced to the future
        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.next_fire_at.is_some(),
            "next_fire_at should be set after firing"
        );
        assert!(
            mission.next_fire_at.unwrap() > chrono::Utc::now(),
            "next_fire_at should be strictly in the future after firing"
        );
    }

    #[tokio::test]
    async fn resume_cron_mission_recomputes_next_fire_at() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "cron resume",
                "periodic goal",
                MissionCadence::Cron {
                    expression: "0 */6 * * *".into(),
                    timezone: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();

        // Pause the mission — this clears it from active list
        mgr.pause_mission(id, "test-user").await.unwrap();

        // Manually set next_fire_at to a stale past value
        {
            let mut missions = store.missions.write().await;
            if let Some(mission) = missions.get_mut(&id) {
                mission.next_fire_at = Some(chrono::Utc::now() - chrono::Duration::hours(24));
            }
        }

        // Resume — should recompute next_fire_at
        mgr.resume_mission(id, "test-user").await.unwrap();

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.next_fire_at.is_some(),
            "resume should recompute next_fire_at for cron missions"
        );
        assert!(
            mission.next_fire_at.unwrap() > chrono::Utc::now(),
            "recomputed next_fire_at should be in the future"
        );
    }

    #[tokio::test]
    async fn update_mission_manual_to_cron_sets_next_fire_at() {
        // Regression: a Manual -> Cron switch left next_fire_at = None and the
        // mission never fired. update_mission must recompute the schedule.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "starts manual",
                "goal",
                MissionCadence::Manual,
                Vec::new(),
            )
            .await
            .unwrap();

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(mission.next_fire_at.is_none());

        mgr.update_mission(
            id,
            "test-user",
            MissionUpdate {
                cadence: Some(MissionCadence::Cron {
                    expression: "0 */6 * * *".into(),
                    timezone: None,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.next_fire_at.is_some(),
            "Manual -> Cron update should compute next_fire_at"
        );
        assert!(
            mission.next_fire_at.unwrap() > chrono::Utc::now(),
            "next_fire_at should be in the future"
        );
    }

    #[tokio::test]
    async fn update_mission_cron_to_manual_clears_next_fire_at() {
        // Regression: a stale next_fire_at must be cleared when switching away
        // from Cron, otherwise the ticker could fire a non-cron mission.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "starts cron",
                "goal",
                MissionCadence::Cron {
                    expression: "0 */6 * * *".into(),
                    timezone: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();
        assert!(
            mgr.get_mission(id)
                .await
                .unwrap()
                .unwrap()
                .next_fire_at
                .is_some()
        );

        mgr.update_mission(
            id,
            "test-user",
            MissionUpdate {
                cadence: Some(MissionCadence::Manual),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.next_fire_at.is_none(),
            "non-cron cadence must clear next_fire_at"
        );
    }

    #[tokio::test]
    async fn create_cron_mission_with_timezone_uses_tz_for_schedule() {
        // Regression: every other cron test in this file passes timezone: None,
        // so the tz path is only exercised at the unit level inside types/mission.
        // This test threads a real ValidTimezone through MissionManager and
        // asserts the resulting next_fire_at differs from the UTC equivalent —
        // proving the bridge router → mission_create → next_cron_fire chain
        // actually honors the user's timezone end-to-end.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();
        let tz = crate::types::mission::ValidTimezone::parse("America/New_York").unwrap();

        let id_tz = mgr
            .create_mission(
                project_id,
                "test-user",
                "tz-aware",
                "fires at 9am NY local",
                MissionCadence::Cron {
                    expression: "0 9 * * *".into(),
                    timezone: Some(tz),
                },
                Vec::new(),
            )
            .await
            .unwrap();

        let id_utc = mgr
            .create_mission(
                project_id,
                "test-user",
                "tz-naive",
                "fires at 9am UTC",
                MissionCadence::Cron {
                    expression: "0 9 * * *".into(),
                    timezone: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();

        let m_tz = mgr.get_mission(id_tz).await.unwrap().unwrap();
        let m_utc = mgr.get_mission(id_utc).await.unwrap().unwrap();
        let next_tz = m_tz.next_fire_at.expect("tz cron should have next_fire_at");
        let next_utc = m_utc
            .next_fire_at
            .expect("utc cron should have next_fire_at");

        // 9am NY = 13:00 or 14:00 UTC depending on DST; 9am UTC = 09:00 UTC.
        use chrono::Timelike;
        assert_ne!(
            next_tz.hour(),
            next_utc.hour(),
            "tz-aware and tz-naive cron schedules must produce different UTC instants"
        );
        let tz_hour = next_tz.hour();
        assert!(
            tz_hour == 13 || tz_hour == 14,
            "9am NY should land on UTC 13 or 14, got {tz_hour}"
        );
        assert_eq!(next_utc.hour(), 9, "9am UTC should land on UTC 9");
    }

    #[tokio::test]
    async fn update_mission_cron_expression_change_recomputes_next_fire_at() {
        // Regression: changing the cron expression must reset the schedule.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "cron edit",
                "goal",
                MissionCadence::Cron {
                    // Year-locked to 2099 so the next fire is deterministically
                    // far in the future regardless of the calendar date the
                    // test runs on. The original `0 0 1 1 *` ("once a year on
                    // Jan 1") was racy around New Year's, when the yearly
                    // schedule's next fire could land within seconds and
                    // invert the `after < before` ordering below.
                    expression: "0 0 0 1 1 * 2099".into(),
                    timezone: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let before = mgr.get_mission(id).await.unwrap().unwrap().next_fire_at;

        mgr.update_mission(
            id,
            "test-user",
            MissionUpdate {
                cadence: Some(MissionCadence::Cron {
                    expression: "* * * * *".into(), // every minute
                    timezone: None,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let after = mgr.get_mission(id).await.unwrap().unwrap().next_fire_at;
        assert!(after.is_some());
        assert_ne!(
            before, after,
            "schedule must be recomputed on cadence change"
        );
        assert!(
            after.unwrap() < before.unwrap(),
            "every-minute schedule should fire sooner than once-a-year"
        );
    }

    #[tokio::test]
    async fn fire_mission_with_corrupt_cron_expression_does_not_orphan_thread() {
        // Regression: previously fire_mission used `?` on next_cron_fire after
        // spawning the thread. A persisted mission with a corrupt cron string
        // would spawn the thread, then abort fire_mission with an Err — leaving
        // the thread running with no entry in thread_history, no incremented
        // budget, and (when also reordered) no outcome watcher installed.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "corrupt cron",
                "goal",
                MissionCadence::Cron {
                    expression: "0 */6 * * *".into(),
                    timezone: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();

        // Capture the original next_fire_at — we expect fire to *preserve* it
        // (rather than replace with None or recompute) when the expression
        // can't be parsed.
        let original_next = mgr
            .get_mission(id)
            .await
            .unwrap()
            .unwrap()
            .next_fire_at
            .expect("create should populate next_fire_at");

        // Corrupt the persisted expression directly in the test store.
        {
            let mut missions = store.missions.write().await;
            if let Some(m) = missions.get_mut(&id)
                && let MissionCadence::Cron {
                    ref mut expression, ..
                } = m.cadence
            {
                *expression = "this is not a cron".to_string();
            }
        }

        // Fire must succeed despite the corrupt expression.
        let thread_id = mgr
            .fire_mission(id, "test-user", None)
            .await
            .expect("fire_mission must not fail on corrupt cron");
        assert!(thread_id.is_some(), "fire should spawn a thread");
        let thread_id = thread_id.unwrap();

        // The mission record must reflect the fire: thread tracked + budget
        // incremented. Without the fix, save_mission was never reached.
        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.thread_history.contains(&thread_id),
            "thread should be recorded in thread_history"
        );
        assert_eq!(
            mission.threads_today, 1,
            "threads_today should be incremented even if next_fire_at couldn't recompute"
        );
        // next_fire_at should be preserved (not cleared) since we couldn't
        // compute a new one.
        assert_eq!(
            mission.next_fire_at,
            Some(original_next),
            "next_fire_at must be preserved when next_cron_fire fails"
        );
    }

    #[tokio::test]
    async fn resume_mission_preserves_concurrent_field_changes() {
        // Regression: resume_mission used to do update_mission_status() then a
        // separate load+save round-trip to recompute next_fire_at. Now it does
        // a single mutate-and-save with the mission already loaded for the
        // ownership check, eliminating the extra interleave window.
        //
        // We can't deterministically exercise the TOCTOU window in a unit
        // test, but we can assert the new contract: resume_mission writes the
        // mission's other fields (e.g. threads_today) faithfully and does not
        // depend on a separate update_mission_status round-trip succeeding.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "test-user",
                "resume preserve",
                "goal",
                MissionCadence::Cron {
                    expression: "0 */6 * * *".into(),
                    timezone: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();

        mgr.pause_mission(id, "test-user").await.unwrap();

        // Simulate a concurrent writer that bumps threads_today between pause
        // and resume. With the old two-write resume path, the second
        // load+save could clobber this. With the single-save path it cannot
        // be clobbered by THIS resume call.
        {
            let mut missions = store.missions.write().await;
            if let Some(m) = missions.get_mut(&id) {
                m.threads_today = 7;
                m.goal = "concurrently updated goal".to_string();
            }
        }

        mgr.resume_mission(id, "test-user").await.unwrap();

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(mission.status, MissionStatus::Active);
        // The concurrent update happened *before* resume_mission's load, so
        // the resume should observe and preserve those values rather than
        // resetting to creation-time defaults.
        assert_eq!(
            mission.threads_today, 7,
            "resume must not reset threads_today to a stale value"
        );
        assert_eq!(
            mission.goal, "concurrently updated goal",
            "resume must not clobber goal updated before its load"
        );
        assert!(
            mission.next_fire_at.is_some(),
            "resume should still recompute next_fire_at for cron"
        );
    }

    #[tokio::test]
    async fn ensure_mission_by_metadata_with_cron_cadence_computes_next_fire_at() {
        // Regression: ensure_mission_by_metadata used to construct
        // Mission::new + save_mission directly, bypassing the next_fire_at
        // computation that create_mission performs. Today every caller passes
        // OnSystemEvent so the bug is latent, but a future caller passing
        // Cron would silently re-introduce the original `next_fire_at = None`
        // bug that #1944 fixes.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .ensure_mission_by_metadata(
                project_id,
                "test-user",
                "synthetic_cron",
                "synthetic-cron",
                "synthetic goal",
                MissionCadence::Cron {
                    expression: "0 9 * * *".into(),
                    timezone: None,
                },
                "synthetic criteria",
                3,
            )
            .await
            .unwrap();

        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(
            mission.next_fire_at.is_some(),
            "ensure_mission_by_metadata must compute next_fire_at for Cron cadence"
        );
        assert!(
            mission.next_fire_at.unwrap() > chrono::Utc::now(),
            "next_fire_at must be in the future"
        );
    }

    #[test]
    fn conversation_insights_count_only_done_threads() {
        assert!(should_count_for_conversation_insights(ThreadState::Done));
        assert!(!should_count_for_conversation_insights(ThreadState::Failed));
    }

    #[test]
    fn build_skill_gap_payload_uses_active_skill_provenance() {
        let project_id = ProjectId::new();
        let mut thread = Thread::new(
            "repair a github workflow",
            ThreadType::Foreground,
            project_id,
            "alice",
            ThreadConfig::default(),
        );
        thread.state = ThreadState::Done;
        let skill_doc_id = DocId::new();
        thread
            .set_active_skills(&[ActiveSkillProvenance {
                doc_id: skill_doc_id,
                name: "github-pr-workflow".to_string(),
                version: 3,
                snippet_names: vec!["list_prs".to_string()],
                force_activated: false,
            }])
            .unwrap();
        thread.add_event(crate::types::event::EventKind::ActionFailed {
            step_id: StepId::new(),
            action_name: "shell".to_string(),
            call_id: "call_1".to_string(),
            error: "gh auth status: not authenticated".to_string(),
            params_summary: None,
        });

        let trace = crate::executor::trace::build_trace(&thread);
        let active_skills = thread.active_skills();
        let payload = build_skill_gap_payload(&thread, &trace, &active_skills).unwrap();

        assert_eq!(
            payload["active_skills"][0]["doc_id"],
            serde_json::Value::String(skill_doc_id.0.to_string())
        );
        let hints = payload["repair_hints"].as_array().unwrap();
        assert!(
            hints
                .iter()
                .any(|hint| hint.as_str() == Some("missing_prerequisite")),
            "repair hints should include missing_prerequisite: {payload}"
        );
    }

    #[test]
    fn build_skill_gap_payload_preserves_recoverable_auth_prerequisite_hints() {
        let project_id = ProjectId::new();
        let mut thread = Thread::new(
            "repair a github workflow",
            ThreadType::Foreground,
            project_id,
            "alice",
            ThreadConfig::default(),
        );
        thread.state = ThreadState::Done;
        thread
            .set_active_skills(&[ActiveSkillProvenance {
                doc_id: DocId::new(),
                name: "github-pr-workflow".to_string(),
                version: 3,
                snippet_names: vec![],
                force_activated: false,
            }])
            .unwrap();
        thread.add_event(crate::types::event::EventKind::ActionFailed {
            step_id: StepId::new(),
            action_name: "shell".to_string(),
            call_id: "call_1".to_string(),
            error: "authentication required for credential github".to_string(),
            params_summary: None,
        });

        let trace = crate::executor::trace::build_trace(&thread);
        let payload = build_skill_gap_payload(&thread, &trace, &thread.active_skills()).unwrap();
        let hints = payload["repair_hints"].as_array().unwrap();

        assert!(
            hints
                .iter()
                .any(|hint| hint.as_str() == Some("missing_prerequisite")),
            "recoverable auth failures should still produce missing_prerequisite: {payload}"
        );
    }

    #[test]
    fn learning_terminal_state_accepts_failed_threads() {
        let failed_event = crate::types::event::EventKind::StateChanged {
            from: ThreadState::Running,
            to: ThreadState::Failed,
            reason: Some("boom".into()),
        };
        assert_eq!(
            learning_terminal_state(&failed_event),
            Some(ThreadState::Failed)
        );

        let done_event = crate::types::event::EventKind::StateChanged {
            from: ThreadState::Completed,
            to: ThreadState::Done,
            reason: None,
        };
        assert_eq!(
            learning_terminal_state(&done_event),
            Some(ThreadState::Done)
        );
    }

    #[test]
    fn thread_completed_successfully_requires_done_without_action_failures() {
        let project_id = ProjectId::new();

        let mut clean_thread = Thread::new(
            "clean success",
            ThreadType::Foreground,
            project_id,
            "alice",
            ThreadConfig::default(),
        );
        clean_thread.state = ThreadState::Done;
        let clean_trace = crate::executor::trace::build_trace(&clean_thread);
        assert!(thread_completed_successfully(&clean_thread, &clean_trace));

        let mut failing_thread = Thread::new(
            "tool failure",
            ThreadType::Foreground,
            project_id,
            "alice",
            ThreadConfig::default(),
        );
        failing_thread.state = ThreadState::Done;
        failing_thread.add_event(crate::types::event::EventKind::ActionFailed {
            step_id: StepId::new(),
            action_name: "shell".to_string(),
            call_id: "call_1".to_string(),
            error: "gh auth status: not authenticated".to_string(),
            params_summary: Some("gh auth status".to_string()),
        });
        let failing_trace = crate::executor::trace::build_trace(&failing_thread);
        assert!(!thread_completed_successfully(
            &failing_thread,
            &failing_trace
        ));
    }

    #[tokio::test]
    async fn process_skill_repair_output_updates_skill_and_records_repair() {
        let store = Arc::new(TestStore::new());
        let project_id = ProjectId::new();
        let skill_doc = make_skill_doc(project_id, "alice", "github-pr-workflow");
        let skill_doc_id = skill_doc.id;
        store.save_memory_doc(&skill_doc).await.unwrap();

        let mut mission = Mission::new(
            project_id,
            "alice",
            "skill-repair",
            SKILL_REPAIR_GOAL,
            MissionCadence::Manual,
        );
        mission.metadata = serde_json::json!({"skill_repair": true});
        mission.last_trigger_payload = Some(serde_json::json!({
            "source_thread_id": "thread-123",
            "active_skills": [{
                "doc_id": skill_doc_id,
                "name": "github-pr-workflow",
                "version": 1,
                "snippet_names": [],
                "force_activated": false
            }]
        }));

        let response = serde_json::json!({
            "doc_id": skill_doc_id,
            "repair_type": "missing_verification",
            "summary": "Added a smoke-test step after the gh command.",
            "updated_content": "1. Run `gh auth status`\n2. Run the PR command\n3. Verify with `gh pr view`",
            "description": "GitHub PR workflow with auth and verification",
        })
        .to_string();

        process_skill_repair_output(&(store.clone() as Arc<dyn Store>), &mission, &response)
            .await
            .unwrap();

        let updated = store.load_memory_doc(skill_doc_id).await.unwrap().unwrap();
        let meta: V2SkillMetadata = serde_json::from_value(updated.metadata).unwrap();
        assert_eq!(meta.version, 2);
        assert_eq!(meta.parent_version, Some(1));
        assert_eq!(
            updated.content,
            "1. Run `gh auth status`\n2. Run the PR command\n3. Verify with `gh pr view`"
        );
        assert_eq!(meta.repairs.len(), 1);
        assert_eq!(
            meta.repairs[0].repair_type,
            SkillRepairType::MissingVerification
        );
        assert_eq!(
            meta.repairs[0].source_thread_id.as_deref(),
            Some("thread-123")
        );
        assert_eq!(meta.revisions.len(), 1);
        assert_eq!(meta.revisions[0].content, "Original skill content");
    }

    #[tokio::test]
    async fn process_skill_repair_output_rejects_stale_trigger_version() {
        let store = Arc::new(TestStore::new());
        let project_id = ProjectId::new();
        let mut skill_doc = make_skill_doc(project_id, "alice", "github-pr-workflow");
        let skill_doc_id = skill_doc.id;
        skill_doc.content = "Skill content already updated to v2".to_string();
        let mut meta: V2SkillMetadata = serde_json::from_value(skill_doc.metadata.clone()).unwrap();
        meta.version = 2;
        meta.parent_version = Some(1);
        skill_doc.metadata = serde_json::to_value(&meta).unwrap();
        store.save_memory_doc(&skill_doc).await.unwrap();

        let mut mission = Mission::new(
            project_id,
            "alice",
            "skill-repair",
            SKILL_REPAIR_GOAL,
            MissionCadence::Manual,
        );
        mission.metadata = serde_json::json!({"skill_repair": true});
        mission.last_trigger_payload = Some(serde_json::json!({
            "source_thread_id": "thread-123",
            "active_skills": [{
                "doc_id": skill_doc_id,
                "name": "github-pr-workflow",
                "version": 1,
                "snippet_names": [],
                "force_activated": false
            }]
        }));

        let response = serde_json::json!({
            "doc_id": skill_doc_id,
            "repair_type": "missing_verification",
            "summary": "Stale repair output.",
            "updated_content": "1. Run the stale command\n2. Verify it"
        })
        .to_string();

        let err =
            process_skill_repair_output(&(store.clone() as Arc<dyn Store>), &mission, &response)
                .await
                .unwrap_err();
        match err {
            EngineError::Skill { reason } => assert!(
                reason.contains("version conflict"),
                "expected version conflict, got: {reason}"
            ),
            other => panic!("expected skill error, got: {other:?}"),
        }

        let updated = store.load_memory_doc(skill_doc_id).await.unwrap().unwrap();
        let updated_meta: V2SkillMetadata = serde_json::from_value(updated.metadata).unwrap();
        assert_eq!(updated.content, "Skill content already updated to v2");
        assert_eq!(updated_meta.version, 2);
        assert!(updated_meta.repairs.is_empty());
    }

    #[tokio::test]
    async fn process_skill_repair_output_rejects_empty_content() {
        let store = Arc::new(TestStore::new());
        let project_id = ProjectId::new();
        let skill_doc = make_skill_doc(project_id, "alice", "github-pr-workflow");
        let skill_doc_id = skill_doc.id;
        store.save_memory_doc(&skill_doc).await.unwrap();

        let mut mission = Mission::new(
            project_id,
            "alice",
            "skill-repair",
            SKILL_REPAIR_GOAL,
            MissionCadence::Manual,
        );
        mission.metadata = serde_json::json!({"skill_repair": true});
        mission.last_trigger_payload = Some(serde_json::json!({
            "source_thread_id": "thread-123",
            "active_skills": [{
                "doc_id": skill_doc_id,
                "name": "github-pr-workflow",
                "version": 1,
                "snippet_names": [],
                "force_activated": false
            }]
        }));

        let response = serde_json::json!({
            "doc_id": skill_doc_id,
            "repair_type": "missing_verification",
            "summary": "This should be rejected.",
            "updated_content": "   "
        })
        .to_string();

        let err =
            process_skill_repair_output(&(store.clone() as Arc<dyn Store>), &mission, &response)
                .await
                .unwrap_err();
        match err {
            EngineError::Skill { reason } => assert!(
                reason.contains("empty updated_content"),
                "expected empty-content validation, got: {reason}"
            ),
            other => panic!("expected skill error, got: {other:?}"),
        }

        let updated = store.load_memory_doc(skill_doc_id).await.unwrap().unwrap();
        let updated_meta: V2SkillMetadata = serde_json::from_value(updated.metadata).unwrap();
        assert_eq!(updated.content, "Original skill content");
        assert_eq!(updated_meta.version, 1);
        assert!(updated_meta.repairs.is_empty());
    }

    #[tokio::test]
    async fn process_skill_repair_output_rejects_shared_skill_updates() {
        let store = Arc::new(TestStore::new());
        let project_id = ProjectId::new();
        let skill_doc = make_skill_doc(project_id, shared_owner_id(), "github-pr-workflow");
        let skill_doc_id = skill_doc.id;
        store.save_memory_doc(&skill_doc).await.unwrap();

        let mut mission = Mission::new(
            project_id,
            "alice",
            "skill-repair",
            SKILL_REPAIR_GOAL,
            MissionCadence::Manual,
        );
        mission.metadata = serde_json::json!({"skill_repair": true});
        mission.last_trigger_payload = Some(serde_json::json!({
            "source_thread_id": "thread-123",
            "active_skills": [{
                "doc_id": skill_doc_id,
                "name": "github-pr-workflow",
                "version": 1,
                "snippet_names": [],
                "force_activated": false
            }]
        }));

        let response = serde_json::json!({
            "doc_id": skill_doc_id,
            "repair_type": "missing_verification",
            "summary": "Attempted shared skill update.",
            "updated_content": "1. Verify auth\n2. Run the command"
        })
        .to_string();

        let err =
            process_skill_repair_output(&(store.clone() as Arc<dyn Store>), &mission, &response)
                .await
                .unwrap_err();
        match err {
            EngineError::AccessDenied { user_id, entity } => {
                assert_eq!(user_id, "alice");
                assert!(entity.contains(&skill_doc_id.0.to_string()));
            }
            other => panic!("expected access denied, got: {other:?}"),
        }

        let unchanged = store.load_memory_doc(skill_doc_id).await.unwrap().unwrap();
        let meta: V2SkillMetadata = serde_json::from_value(unchanged.metadata).unwrap();
        assert_eq!(unchanged.content, "Original skill content");
        assert_eq!(meta.version, 1);
        assert!(meta.repairs.is_empty());
    }

    #[tokio::test]
    async fn dedup_event_does_not_evict_entries_from_other_missions() {
        // Regression for the cross-mission dedup window collision: a
        // mission with a *short* window must not be able to evict a fresh
        // entry belonging to a mission with a *longer* window.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);

        let mission_a = MissionId::new();
        let mission_b = MissionId::new();

        // Mission B (long window) gets a stale entry for key "y" — set it
        // 120 seconds in the past so a 60s-window check would consider it
        // expired but a 3600s-window check still considers it fresh.
        {
            let mut table = mgr.dedup_table.write().await;
            table.insert(
                (mission_b, "y".to_string()),
                chrono::Utc::now() - chrono::Duration::seconds(120),
            );
        }

        // Mission A (short 60s window) fires for an unrelated key.
        let first = mgr.dedup_event(mission_a, "x", 60).await;
        assert!(!first, "first sighting of (A, x) should not be flagged");

        // Mission B's entry must survive — its own window is 3600s, and
        // 120s < 3600s, so the next dedup call from B for "y" should still
        // see it as a duplicate.
        let b_again = mgr.dedup_event(mission_b, "y", 3600).await;
        assert!(
            b_again,
            "(B, y) is 120s old with a 3600s window — must still register as duplicate after A's call"
        );
    }

    #[tokio::test]
    async fn user_rate_slot_not_consumed_by_failed_fire() {
        // Regression for the rate-limiter self-DoS: when fire_mission
        // refuses (e.g. budget/concurrent gate) the per-user slot must
        // remain available so sustained refusals don't lock the user out.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);

        // Sanity: an empty window allows fires.
        assert!(mgr.check_user_rate("alice").await);

        // Drive a fire that's guaranteed to early-out before record_user_rate.
        // The simplest deterministic refusal is `fire_mission` against a
        // mission whose owner doesn't match — that returns AccessDenied
        // before reaching the rate check, so it doesn't exercise the
        // rate-limit path. Instead, drive `record_user_rate` and
        // `check_user_rate` directly to pin the contract: a check that
        // doesn't get followed by a record leaves the slot free.
        let allowed_before = mgr.check_user_rate("alice").await;
        assert!(allowed_before);

        // Snapshot the queue size — must be unchanged after a check-only.
        let snapshot_after_check = {
            let log = mgr.user_fire_log.read().await;
            log.get("alice").map(|q| q.len()).unwrap_or(0)
        };
        assert_eq!(
            snapshot_after_check, 0,
            "check_user_rate must not consume a slot on its own"
        );

        // After a successful fire would have called record_user_rate the
        // queue grows by exactly one.
        mgr.record_user_rate("alice").await;
        let snapshot_after_record = {
            let log = mgr.user_fire_log.read().await;
            log.get("alice").map(|q| q.len()).unwrap_or(0)
        };
        assert_eq!(
            snapshot_after_record, 1,
            "record_user_rate must append exactly one entry"
        );
    }

    #[test]
    fn build_skill_gap_payload_skips_read_only_shell_workflows() {
        let project_id = ProjectId::new();
        let mut thread = Thread::new(
            "inspect github pull requests",
            ThreadType::Foreground,
            project_id,
            "alice",
            ThreadConfig::default(),
        );
        thread.state = ThreadState::Done;
        thread
            .set_active_skills(&[ActiveSkillProvenance {
                doc_id: DocId::new(),
                name: "github-pr-workflow".to_string(),
                version: 1,
                snippet_names: vec![],
                force_activated: false,
            }])
            .unwrap();
        thread.add_event(crate::types::event::EventKind::ActionExecuted {
            step_id: StepId::new(),
            action_name: "shell".to_string(),
            call_id: "call_1".to_string(),
            params_summary: Some("gh pr list --repo nearai/ironclaw".to_string()),
            duration_ms: 15,
        });

        let trace = crate::executor::trace::build_trace(&thread);
        assert!(
            build_skill_gap_payload(&thread, &trace, &thread.active_skills()).is_none(),
            "read-only shell workflows should not trigger skill repair"
        );
    }

    // ── next_cron_fire_required + cooldown regression tests ──────

    /// A 7-field cron expression year-locked to a year that's already in the
    /// past. `cron::Schedule` parses it cleanly but `upcoming(...).next()`
    /// returns `None`, which is exactly the `Ok(None)` case the
    /// `next_cron_fire_required` helper guards against.
    const PAST_YEAR_CRON: &str = "0 0 0 1 1 * 2020";

    #[tokio::test]
    async fn create_mission_rejects_unschedulable_cron() {
        // Regression: previously `create_mission` accepted Ok(None) and
        // persisted an Active mission with `next_fire_at = None` — the
        // exact failure mode of #1944. Now it must fail fast.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let err = mgr
            .create_mission(
                project_id,
                "alice",
                "unschedulable",
                "g",
                MissionCadence::Cron {
                    expression: PAST_YEAR_CRON.into(),
                    timezone: None,
                },
                vec![],
            )
            .await
            .expect_err("create_mission must reject cron with no upcoming fire time");

        assert!(
            matches!(err, EngineError::InvalidCadence { .. }),
            "expected InvalidCadence, got: {err:?}"
        );

        // No mission should be persisted, no entry in active.
        assert!(store.missions.read().await.is_empty());
    }

    #[tokio::test]
    async fn update_mission_rejects_switch_to_unschedulable_cron() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "alice",
                "manual-then-cron",
                "g",
                MissionCadence::Manual,
                vec![],
            )
            .await
            .unwrap();

        let err = mgr
            .update_mission(
                id,
                "alice",
                MissionUpdate {
                    cadence: Some(MissionCadence::Cron {
                        expression: PAST_YEAR_CRON.into(),
                        timezone: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .expect_err("update_mission must reject cron with no upcoming fire time");

        assert!(matches!(err, EngineError::InvalidCadence { .. }));
        // Original Manual cadence should be preserved on the persisted record.
        let reloaded = mgr.get_mission(id).await.unwrap().unwrap();
        assert!(matches!(reloaded.cadence, MissionCadence::Manual));
    }

    #[tokio::test]
    async fn resume_mission_rejects_unschedulable_cron() {
        // Build a paused cron mission whose schedule is fine, then mutate the
        // persisted record to a year-locked expression and try to resume.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "alice",
                "resume-bad",
                "g",
                MissionCadence::Cron {
                    expression: "0 9 * * *".into(),
                    timezone: None,
                },
                vec![],
            )
            .await
            .unwrap();
        mgr.pause_mission(id, "alice").await.unwrap();

        // Tamper with the persisted cadence to simulate a stored mission that
        // can no longer fire (e.g. operator edited the database, or year-locked
        // schedule rolled past).
        {
            let mut missions = store.missions.write().await;
            if let Some(m) = missions.get_mut(&id) {
                m.cadence = MissionCadence::Cron {
                    expression: PAST_YEAR_CRON.into(),
                    timezone: None,
                };
            }
        }

        let err = mgr
            .resume_mission(id, "alice")
            .await
            .expect_err("resume_mission must reject cron with no upcoming fire time");
        assert!(matches!(err, EngineError::InvalidCadence { .. }));

        // Mission must remain paused — resume failed before any state change.
        let reloaded = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(reloaded.status, MissionStatus::Paused);
    }

    #[tokio::test]
    async fn pause_and_complete_drop_cooldown_entry() {
        // Regression: `last_fire_attempt` was previously only ever inserted,
        // never pruned. Pausing or completing a mission must drop its
        // cooldown entry so the in-memory map can't grow unbounded.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Mission A — paused after a fire.
        let id_a = mgr
            .create_mission(
                project_id,
                "alice",
                "pause-cleanup",
                "g",
                MissionCadence::Cron {
                    expression: "* * * * *".into(),
                    timezone: None,
                },
                vec![],
            )
            .await
            .unwrap();
        mgr.fire_mission(id_a, "alice", None).await.unwrap();
        assert!(mgr.last_fire_attempt.read().await.contains_key(&id_a));
        mgr.pause_mission(id_a, "alice").await.unwrap();
        assert!(
            !mgr.last_fire_attempt.read().await.contains_key(&id_a),
            "pause_mission must drop the cooldown entry"
        );

        // Mission B — completed after a fire.
        let id_b = mgr
            .create_mission(
                project_id,
                "alice",
                "complete-cleanup",
                "g",
                MissionCadence::Cron {
                    expression: "* * * * *".into(),
                    timezone: None,
                },
                vec![],
            )
            .await
            .unwrap();
        mgr.fire_mission(id_b, "alice", None).await.unwrap();
        assert!(mgr.last_fire_attempt.read().await.contains_key(&id_b));
        mgr.complete_mission(id_b).await.unwrap();
        assert!(
            !mgr.last_fire_attempt.read().await.contains_key(&id_b),
            "complete_mission must drop the cooldown entry"
        );
    }

    #[tokio::test]
    async fn tick_cooldown_suppresses_re_fire_on_save_failure() {
        // Regression for the runaway-re-fire concern: when save_mission fails
        // after a successful spawn, the persisted `next_fire_at` AND
        // `last_fire_at` stay at their pre-fire values, but the in-memory
        // `last_fire_attempt[mid]` is set to the new fire instant. The
        // mismatch between in-memory and persisted `last_fire_at` is what
        // tells tick to arm the cooldown — without that signal every
        // subsequent tick would re-fire the same mission and spawn
        // duplicate threads up to the daily budget.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "alice",
                "cooldown",
                "g",
                MissionCadence::Cron {
                    expression: "* * * * *".into(),
                    timezone: None,
                },
                vec![],
            )
            .await
            .unwrap();

        // First fire arms the in-memory `last_fire_attempt` map.
        let first = mgr.fire_mission(id, "alice", None).await.unwrap();
        assert!(first.is_some(), "first fire should spawn a thread");

        // Simulate the post-save-failure state explicitly: rewind
        // `next_fire_at` into the past, reset `threads_today` so the budget
        // can't be what's blocking, AND clobber `last_fire_at` so it no
        // longer matches the in-memory `last_fire_attempt[mid]` instant.
        // Together these mimic exactly the state a failed `save_mission`
        // call would leave: in-memory recorded the fire, the store didn't.
        {
            let mut missions = store.missions.write().await;
            let mission = missions.get_mut(&id).unwrap();
            mission.next_fire_at = Some(chrono::Utc::now() - chrono::Duration::seconds(120));
            mission.threads_today = 0;
            mission.last_fire_at = None;
        }

        // tick must suppress the second fire — it can prove the persisted
        // record is stale because in-memory last_fire_attempt holds an
        // instant the persisted `last_fire_at` doesn't.
        let spawned = mgr.tick("alice").await.unwrap();
        assert!(
            spawned.is_empty(),
            "tick must skip mission whose persisted last_fire_at is stale, got: {spawned:?}"
        );
    }

    #[tokio::test]
    async fn fire_mission_arms_cooldown_before_save_mission() {
        // Race regression: a concurrent tick observing the state between
        // `save_mission` completion and the in-memory cooldown insert would
        // see no cooldown entry, evaluate the mismatch check to false, and
        // (if next_fire_at is in the past) re-fire immediately. Fix: insert
        // the cooldown entry BEFORE calling save_mission.
        //
        // We verify the order by gating save_mission with a oneshot channel
        // and asserting `last_fire_attempt[mid]` is already populated while
        // save is still in flight.
        use tokio::sync::Notify;

        let store = Arc::new(TestStore::new());
        let mgr = Arc::new(make_mission_manager(Arc::clone(&store) as Arc<dyn Store>));
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "alice",
                "race-test",
                "g",
                MissionCadence::Cron {
                    expression: "0 9 * * *".into(),
                    timezone: None,
                },
                vec![],
            )
            .await
            .unwrap();

        // Block the next save_mission. The first save_mission call below
        // (from create_mission's path) already happened — `block_next_save_mission`
        // installs the gate AFTER create, so it only catches fire_mission's save.
        let release = store.block_next_save_mission().await;
        let started = Arc::new(Notify::new());
        let started_clone = Arc::clone(&started);
        let store_clone = Arc::clone(&store);
        // Spawn a watcher that translates `save_mission_started` into our own
        // `started` notification. We can't share the TestStore's Notify across
        // tasks via `notified()` cleanly without a permit, so wrap it.
        tokio::spawn(async move {
            store_clone.save_mission_started.notified().await;
            started_clone.notify_one();
        });

        // Spawn fire_mission in a task — it will block inside save_mission.
        let mgr_clone = Arc::clone(&mgr);
        let fire_task = tokio::spawn(async move {
            mgr_clone
                .fire_mission(id, "alice", None)
                .await
                .expect("fire should succeed once save is unblocked")
        });

        // Wait until save_mission has begun (inside the gate).
        started.notified().await;

        // At this point save_mission is parked. The cooldown MUST already be
        // armed because the fix inserts before save.
        assert!(
            mgr.last_fire_attempt.read().await.contains_key(&id),
            "last_fire_attempt[mid] must be populated before save_mission begins; \
             a concurrent tick in this window would otherwise see no cooldown entry"
        );

        // Unblock save and let fire_mission complete.
        release.send(()).unwrap();
        let thread_id = fire_task.await.unwrap();
        assert!(thread_id.is_some(), "fire should spawn a thread");
    }

    #[tokio::test]
    async fn tick_does_not_re_fire_corrupted_cron_within_cooldown_window() {
        // Regression: when `next_cron_fire(expression)` returns Err inside
        // fire_mission (corrupted persisted expression), the previous code
        // stamped `last_fire_at = fire_instant` anyway. Save then succeeded
        // with last_fire_at matching the in-memory value, so tick's
        // mismatch detector saw "save succeeded" and the cooldown was
        // never armed. With `next_fire_at` still in the past (preserved
        // because the cron crate couldn't compute a new value), every
        // subsequent tick re-fired the same mission until `max_threads_per_day`
        // was exhausted.
        //
        // Fix: only stamp `last_fire_at = fire_instant` when cron actually
        // advanced. On a parse error, leave persisted last_fire_at at its
        // OLD value so the in-memory vs persisted mismatch arms the
        // cooldown via the same code path as a save failure.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "alice",
                "corrupt-cron",
                "g",
                MissionCadence::Cron {
                    expression: "* * * * *".into(),
                    timezone: None,
                },
                vec![],
            )
            .await
            .unwrap();

        // Corrupt the persisted expression. fire_mission's cron advance
        // call will fail and (with the fix) leave last_fire_at at the OLD
        // value the test fixture started with.
        {
            let mut missions = store.missions.write().await;
            let mission = missions.get_mut(&id).unwrap();
            if let MissionCadence::Cron {
                ref mut expression, ..
            } = mission.cadence
            {
                *expression = "this is not a cron".to_string();
            }
            // Force next_fire_at into the past so should_fire is true.
            mission.next_fire_at = Some(chrono::Utc::now() - chrono::Duration::seconds(60));
        }

        // First fire spawns a thread successfully despite the corrupt cron
        // (regression test `fire_mission_with_corrupt_cron_expression_does_not_orphan_thread`
        // pins this behavior). next_fire_at stays at its past value.
        let first = mgr.fire_mission(id, "alice", None).await.unwrap();
        assert!(
            first.is_some(),
            "first fire should spawn despite corrupt cron"
        );

        // tick must NOT re-fire the corrupted mission. Without the fix the
        // cooldown was not armed (persisted last_fire_at == fire_instant ==
        // in-memory), and tick would call fire_mission again every cycle.
        let spawned = mgr.tick("alice").await.unwrap();
        assert!(
            spawned.is_empty(),
            "tick must not re-fire a mission whose cron advance failed; \
             cooldown should be armed via last_fire_at vs in-memory mismatch, \
             got: {spawned:?}"
        );

        // Sanity: the persisted last_fire_at is still its pre-fire value
        // (None for a freshly-created mission whose first fire failed to
        // advance), confirming the mismatch-arming mechanism.
        let reloaded = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(
            reloaded.last_fire_at, None,
            "last_fire_at must NOT be stamped when cron advance failed"
        );
    }

    #[tokio::test]
    async fn tick_does_not_throttle_high_frequency_cron_after_successful_fire() {
        // Regression for the inverse failure mode: the cooldown must NOT
        // throttle a normally-firing high-frequency cron. An earlier
        // implementation armed the cooldown unconditionally on every
        // successful fire, which silently dropped roughly half of the
        // events for `* * * * *` (every-minute) crons because the 60 s
        // tick interval fell inside the 90 s cooldown window. The fix:
        // only arm the cooldown when the persisted `last_fire_at` does
        // NOT match the in-memory `last_fire_attempt` value — i.e. only
        // in the failed-save regime.
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(
                project_id,
                "alice",
                "high-freq",
                "g",
                MissionCadence::Cron {
                    expression: "* * * * *".into(),
                    timezone: None,
                },
                vec![],
            )
            .await
            .unwrap();

        // First fire records the in-memory cooldown entry AND persists
        // `last_fire_at = fire_instant`. The two values are the same
        // (`fire_mission` uses a single `Utc::now()` for both writes).
        let first = mgr.fire_mission(id, "alice", None).await.unwrap();
        assert!(first.is_some(), "first fire should spawn a thread");

        // Mimic "tick runs ~1 minute later, schedule advanced normally":
        // rewind `next_fire_at` to a moment in the past that is STRICTLY
        // LATER than the fire instant. Crucially, leave `last_fire_at`
        // alone — it still equals `last_fire_attempt[mid]`, so the
        // cooldown's mismatch detector says "save succeeded, do not
        // throttle." Reset `threads_today` so the daily budget isn't
        // what's blocking.
        {
            let mut missions = store.missions.write().await;
            let mission = missions.get_mut(&id).unwrap();
            // 1 ms earlier than now, but still later than the original
            // fire instant since fire_mission ran microseconds ago.
            mission.next_fire_at = Some(chrono::Utc::now() - chrono::Duration::milliseconds(1));
            mission.threads_today = 0;
        }

        let spawned = mgr.tick("alice").await.unwrap();
        assert_eq!(
            spawned.len(),
            1,
            "tick must fire a high-frequency cron after a successful fire — \
             cooldown must not throttle the success path, got spawned={spawned:?}"
        );
    }
}
