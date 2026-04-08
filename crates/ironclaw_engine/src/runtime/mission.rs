//! Mission manager — orchestrates long-running goals that spawn threads over time.
//!
//! Missions track ongoing objectives and periodically spawn threads to make
//! progress. The manager handles lifecycle (create, pause, resume, complete)
//! and delegates thread spawning to [`ThreadManager`].

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::memory::RetrievalEngine;
use crate::runtime::manager::ThreadManager;
use crate::runtime::messaging::ThreadOutcome;
use crate::traits::store::Store;
use crate::traits::workspace::WorkspaceReader;
use crate::types::error::EngineError;
use crate::types::memory::MemoryDoc;
use crate::types::mission::{Mission, MissionCadence, MissionId, MissionStatus};
use crate::types::project::ProjectId;
use crate::types::shared_owner_id;
use crate::types::thread::{ThreadConfig, ThreadId, ThreadState, ThreadType};

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

impl MissionManager {
    pub fn new(store: Arc<dyn Store>, thread_manager: Arc<ThreadManager>) -> Self {
        let (notification_tx, _) = tokio::sync::broadcast::channel(64);
        Self {
            store,
            thread_manager,
            active: RwLock::new(Vec::new()),
            notification_tx,
            workspace: None,
            dedup_table: RwLock::new(HashMap::new()),
            event_regex_cache: RwLock::new(HashMap::new()),
            user_fire_log: RwLock::new(HashMap::new()),
            rate_limit: FireRateLimit::default(),
            budget_gate: None,
        }
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

    /// Populate the active mission index from persisted mission state.
    pub async fn bootstrap_project(&self, project_id: ProjectId) -> Result<usize, EngineError> {
        // System operation: load all missions for the project regardless of user.
        let missions = self.store.list_all_missions(project_id).await?;
        let active_ids: Vec<MissionId> = missions
            .into_iter()
            .filter(|mission| mission.status == MissionStatus::Active)
            .map(|mission| mission.id)
            .collect();

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

        if !mission.owner_id().is_shared() && !mission.is_owned_by(user_id) {
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
    /// For shared missions, the caller (web handler) must
    /// verify admin role before calling this. The engine only checks ownership.
    pub async fn pause_mission(&self, id: MissionId, user_id: &str) -> Result<(), EngineError> {
        // Validate ownership. Shared missions require admin role (checked by caller).
        if let Some(mission) = self.store.load_mission(id).await?
            && !mission.is_owned_by(user_id)
            && !mission.owner_id().is_shared()
        {
            return Err(EngineError::AccessDenied {
                user_id: user_id.to_string(),
                entity: format!("mission {id}"),
            });
        }
        self.store
            .update_mission_status(id, MissionStatus::Paused)
            .await?;
        self.active.write().await.retain(|mid| *mid != id);
        debug!(mission_id = %id, "mission paused");
        Ok(())
    }

    /// Resume a paused mission.
    ///
    /// For shared missions, the caller (web handler) must
    /// verify admin role before calling this. The engine only checks ownership.
    pub async fn resume_mission(&self, id: MissionId, user_id: &str) -> Result<(), EngineError> {
        // Validate ownership. Shared missions require admin role (checked by caller).
        if let Some(mission) = self.store.load_mission(id).await?
            && !mission.is_owned_by(user_id)
            && !mission.owner_id().is_shared()
        {
            return Err(EngineError::AccessDenied {
                user_id: user_id.to_string(),
                entity: format!("mission {id}"),
            });
        }
        self.store
            .update_mission_status(id, MissionStatus::Active)
            .await?;
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
        // flood the LLM. The check is recorded only when it passes — a
        // refusal does not consume a slot.
        if !self.check_and_record_user_rate(&mission.user_id).await {
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

        // Record the thread + trigger payload in mission history
        let mut updated = mission;
        updated.record_thread(thread_id);
        updated.threads_today += 1;
        updated.last_trigger_payload = trigger_payload;
        updated.last_fire_at = Some(chrono::Utc::now());
        self.store.save_mission(&updated).await?;

        debug!(mission_id = %id, thread_id = %thread_id, "mission fired");
        self.spawn_mission_outcome_watcher(id, thread_id);

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
                self.spawn_mission_outcome_watcher(mission_id, thread_id);
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
    /// 1. **Error diagnosis** — if trace has issues, fires `thread_completed_with_issues`
    /// 2. **Skill extraction** — if thread succeeded with many steps/actions,
    ///    fires `thread_completed_with_learnings`
    /// 3. **Conversation insights** — after every N threads in a conversation,
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
                        // Only react to threads transitioning to Done
                        let is_done = matches!(
                            event.kind,
                            crate::types::event::EventKind::StateChanged {
                                to: crate::types::thread::ThreadState::Done,
                                ..
                            }
                        );
                        if !is_done {
                            continue;
                        }

                        // Load the completed thread
                        let thread = match mgr.store.load_thread(event.thread_id).await {
                            Ok(Some(t)) => t,
                            _ => continue,
                        };

                        // Skip Mission threads (no recursive self-improvement)
                        if thread.thread_type == ThreadType::Mission {
                            continue;
                        }

                        // ── Trigger 1: Error diagnosis ──────────────────
                        let trace = crate::executor::trace::build_trace(&thread);
                        if !trace.issues.is_empty() {
                            let issues: Vec<serde_json::Value> = trace
                                .issues
                                .iter()
                                .map(|i| {
                                    serde_json::json!({
                                        "severity": format!("{:?}", i.severity),
                                        "category": i.category,
                                        "description": i.description,
                                        "step": i.step,
                                    })
                                })
                                .collect();

                            let error_messages: Vec<String> = thread
                                .events
                                .iter()
                                .filter_map(|e| {
                                    if let crate::types::event::EventKind::ActionFailed {
                                        action_name,
                                        error,
                                        ..
                                    } = &e.kind
                                    {
                                        Some(format!("{action_name}: {error}"))
                                    } else {
                                        None
                                    }
                                })
                                .take(10)
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

                        // ── Trigger 2: Skill extraction ──────────────────
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

                        if thread.state == crate::types::thread::ThreadState::Done
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

                        // ── Trigger 3: Conversation insights ────────────
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
                                debug!("event listener: failed to fire conversation insights: {e}");
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

    /// Ensure all three learning missions exist for the given project.
    ///
    /// Creates (if missing) the self-improvement, skill extraction, and
    /// conversation insights missions. This is the preferred entry point —
    /// call once at project bootstrap.
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

        // 2. Skill extraction (formerly playbook extraction)
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

        // 3. Conversation insights
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

        // 4. Expected behavior (user feedback loop)
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

        for mid in active_ids {
            let mission = match self.store.load_mission(mid).await? {
                Some(m) if m.status == MissionStatus::Active => m,
                _ => continue,
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

            // `fire_mission` enforces cooldown_secs and max_concurrent
            // independently of the cron next_fire_at, so a cron mission whose
            // schedule fires faster than its cooldown will simply skip the
            // intervening firings rather than backlog them.
            if let Some(tid) = self.fire_mission(mid, &mission.user_id, None).await? {
                spawned.push(tid);
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
    /// Drops entries older than `window_secs` opportunistically to bound the
    /// table size.
    async fn dedup_event(&self, mission_id: MissionId, dedup_key: &str, window_secs: u64) -> bool {
        if window_secs == 0 {
            return false;
        }
        let now = chrono::Utc::now();
        let window = chrono::Duration::seconds(window_secs as i64);
        let mut table = self.dedup_table.write().await;
        // Opportunistic cleanup: drop entries older than the window so the
        // table doesn't grow without bound.
        table.retain(|_, ts| now.signed_duration_since(*ts) < window);

        let key = (mission_id, dedup_key.to_string());
        if let Some(last) = table.get(&key)
            && now.signed_duration_since(*last) < window
        {
            return true;
        }
        table.insert(key, now);
        false
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

    /// Per-user global rate limiter check. Sliding window of timestamps;
    /// returns `true` if a new fire is allowed and records the timestamp.
    /// Returns `false` if the window is full.
    async fn check_and_record_user_rate(&self, user_id: &str) -> bool {
        let now = chrono::Utc::now();
        let window = chrono::Duration::from_std(self.rate_limit.window)
            .unwrap_or_else(|_| chrono::Duration::seconds(self.rate_limit.window.as_secs() as i64));
        let cutoff = now - window;

        let mut log = self.user_fire_log.write().await;
        let entries = log.entry(user_id.to_string()).or_default();
        // Evict expired entries from the front.
        while entries.front().is_some_and(|ts| *ts < cutoff) {
            entries.pop_front();
        }
        if entries.len() as u32 >= self.rate_limit.max_fires {
            return false;
        }
        entries.push_back(now);
        true
    }

    /// Consult the budget gate (if attached). Returns `true` when the gate
    /// is unattached or explicitly allows the fire.
    async fn budget_allows(&self, user_id: &str, mission_id: MissionId) -> bool {
        match self.budget_gate.as_ref() {
            Some(gate) => gate.allow_mission_fire(user_id, mission_id).await,
            None => true,
        }
    }

    fn spawn_mission_outcome_watcher(&self, mission_id: MissionId, thread_id: ThreadId) {
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
    process_mission_outcome_and_notify(store, mission_id, thread_id, outcome, &notification_tx)
        .await
}

async fn process_mission_outcome_and_notify(
    store: &Arc<dyn Store>,
    mission_id: MissionId,
    thread_id: ThreadId,
    outcome: &ThreadOutcome,
    notification_tx: &tokio::sync::broadcast::Sender<MissionNotification>,
) -> Result<(), EngineError> {
    let mut mission = match store.load_mission(mission_id).await? {
        Some(m) => m,
        None => return Ok(()),
    };

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
        let notification = MissionNotification {
            mission_id,
            mission_name: mission.name.clone(),
            thread_id,
            user_id: mission.user_id.clone(),
            notify_channels: mission.notify_channels.clone(),
            notify_user: mission.notify_user.clone(),
            response: notify_response,
            is_error,
        };
        // Best-effort: ignore send errors (no subscribers = no problem).
        let _ = notification_tx.send(notification);
    }

    mission.updated_at = chrono::Utc::now();
    store.save_mission(&mission).await
}

/// Check if a mission is the self-improvement mission.
fn is_self_improvement_mission(mission: &Mission) -> bool {
    mission
        .metadata
        .get("self_improvement")
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
    use std::time::Duration;

    use crate::capability::lease::LeaseManager;
    use crate::capability::policy::PolicyEngine;
    use crate::capability::registry::CapabilityRegistry;
    use crate::traits::effect::EffectExecutor;
    use crate::traits::llm::{LlmCallConfig, LlmOutput};
    use crate::traits::store::Store;
    use crate::types::capability::{ActionDef, CapabilityLease};
    use crate::types::error::EngineError;
    use crate::types::event::ThreadEvent;
    use crate::types::memory::{DocId, MemoryDoc};
    use crate::types::mission::{Mission, MissionCadence, MissionId, MissionStatus};
    use crate::types::project::{Project, ProjectId};
    use crate::types::step::{ActionResult, LlmResponse, Step, TokenUsage};
    use crate::types::thread::{Thread, ThreadId, ThreadState};

    // ── TestStore — in-memory Store that persists missions ───

    struct TestStore {
        threads: tokio::sync::RwLock<HashMap<ThreadId, Thread>>,
        missions: tokio::sync::RwLock<HashMap<MissionId, Mission>>,
        docs: tokio::sync::RwLock<Vec<MemoryDoc>>,
    }

    impl TestStore {
        fn new() -> Self {
            Self {
                threads: tokio::sync::RwLock::new(HashMap::new()),
                missions: tokio::sync::RwLock::new(HashMap::new()),
                docs: tokio::sync::RwLock::new(Vec::new()),
            }
        }
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

        // Create a cron mission with next_fire_at in the past
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

        // Set next_fire_at to the past so tick() will fire it
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
    async fn system_mission_requires_system_user_to_manage() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        // Create a system mission
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

        // Regular user cannot pause system mission
        let result = mgr.pause_mission(system_id, "alice").await;
        assert!(
            matches!(result.unwrap_err(), EngineError::AccessDenied { .. }),
            "regular user cannot manage system missions"
        );

        // System user can pause (admin path passes "system" as user_id)
        mgr.pause_mission(system_id, "system").await.unwrap();
        let m = mgr.get_mission(system_id).await.unwrap().unwrap();
        assert_eq!(m.status, MissionStatus::Paused);

        // System user can resume
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
}
