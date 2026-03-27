//! Mission manager — orchestrates long-running goals that spawn threads over time.
//!
//! Missions track ongoing objectives and periodically spawn threads to make
//! progress. The manager handles lifecycle (create, pause, resume, complete)
//! and delegates thread spawning to [`ThreadManager`].

use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::memory::RetrievalEngine;
use crate::runtime::manager::ThreadManager;
use crate::runtime::messaging::ThreadOutcome;
use crate::traits::store::Store;
use crate::types::error::EngineError;
use crate::types::memory::MemoryDoc;
use crate::types::mission::{Mission, MissionCadence, MissionId, MissionStatus};
use crate::types::project::ProjectId;
use crate::types::thread::{ThreadConfig, ThreadId, ThreadType};

/// Manages mission lifecycle and thread spawning.
pub struct MissionManager {
    store: Arc<dyn Store>,
    thread_manager: Arc<ThreadManager>,
    /// Active missions indexed by ID for quick lookup.
    active: RwLock<Vec<MissionId>>,
}

impl MissionManager {
    pub fn new(store: Arc<dyn Store>, thread_manager: Arc<ThreadManager>) -> Self {
        Self {
            store,
            thread_manager,
            active: RwLock::new(Vec::new()),
        }
    }

    /// Populate the active mission index from persisted mission state.
    pub async fn bootstrap_project(&self, project_id: ProjectId) -> Result<usize, EngineError> {
        let missions = self.store.list_missions(project_id).await?;
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
        name: impl Into<String>,
        goal: impl Into<String>,
        cadence: MissionCadence,
    ) -> Result<MissionId, EngineError> {
        let mission = Mission::new(project_id, name, goal, cadence);
        let id = mission.id;
        self.store.save_mission(&mission).await?;
        self.active.write().await.push(id);
        debug!(mission_id = %id, "mission created");
        Ok(id)
    }

    /// Pause an active mission. No new threads will be spawned.
    pub async fn pause_mission(&self, id: MissionId) -> Result<(), EngineError> {
        self.store
            .update_mission_status(id, MissionStatus::Paused)
            .await?;
        self.active.write().await.retain(|mid| *mid != id);
        debug!(mission_id = %id, "mission paused");
        Ok(())
    }

    /// Resume a paused mission.
    pub async fn resume_mission(&self, id: MissionId) -> Result<(), EngineError> {
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

        if mission.is_terminal() {
            warn!(mission_id = %id, status = ?mission.status, "cannot fire terminal mission");
            return Ok(None);
        }

        // Check daily budget
        if mission.max_threads_per_day > 0 && mission.threads_today >= mission.max_threads_per_day {
            debug!(mission_id = %id, "daily thread budget exhausted");
            return Ok(None);
        }

        // Build meta-prompt from mission state + project docs
        let retrieval = RetrievalEngine::new(Arc::clone(&self.store));
        let project_docs = retrieval
            .retrieve_context(mission.project_id, &mission.goal, 10)
            .await
            .unwrap_or_default();
        let meta_prompt = build_meta_prompt(&mission, &project_docs, &trigger_payload);

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
                    .resume_thread(thread_id, user_id.to_string(), None, None)
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
                        warn!("cron ticker error: {e}");
                    }
                    _ => {}
                }
            }
        });
    }

    /// List all missions in a project.
    pub async fn list_missions(&self, project_id: ProjectId) -> Result<Vec<Mission>, EngineError> {
        self.store.list_missions(project_id).await
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

            let matches = match &mission.cadence {
                MissionCadence::OnSystemEvent {
                    source: s,
                    event_type: et,
                } => s == source && et == event_type,
                _ => false,
            };

            if matches && let Some(tid) = self.fire_mission(mid, user_id, payload.clone()).await? {
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
    /// 2. **Playbook extraction** — if thread succeeded with many steps/actions,
    ///    fires `thread_completed_with_learnings`
    /// 3. **Conversation insights** — after every N threads in a conversation,
    ///    fires `conversation_insights_due`
    pub fn start_event_listener(self: &Arc<Self>, user_id: String) {
        let mgr = Arc::clone(self);
        let mut rx = mgr.thread_manager.subscribe_events();

        /// Minimum steps for a thread to be a playbook candidate.
        const PLAYBOOK_MIN_STEPS: usize = 5;
        /// Minimum distinct action executions for playbook extraction.
        const PLAYBOOK_MIN_ACTIONS: usize = 3;
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
                                    &user_id,
                                    Some(payload),
                                )
                                .await
                            {
                                warn!("event listener: failed to fire error diagnosis: {e}");
                            }
                        }

                        // ── Trigger 2: Playbook extraction ──────────────
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
                            && trace.issues.iter().all(|i| {
                                i.severity != crate::executor::trace::IssueSeverity::Error
                            })
                            && thread.step_count >= PLAYBOOK_MIN_STEPS
                            && action_count >= PLAYBOOK_MIN_ACTIONS
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
                                    &user_id,
                                    Some(payload),
                                )
                                .await
                            {
                                warn!("event listener: failed to fire skill extraction: {e}");
                            }
                        }

                        // ── Trigger 3: Conversation insights ────────────
                        // Use the thread's project_id as a proxy for conversation scope.
                        let conv_key = thread.project_id.0.to_string();
                        let count = conv_thread_counts
                            .entry(conv_key.clone())
                            .or_insert(0);
                        *count += 1;

                        if (*count).is_multiple_of(CONVERSATION_INSIGHTS_INTERVAL) {
                            // Collect recent thread goals for context
                            let thread_goals: Vec<String> = match mgr
                                .store
                                .list_threads(thread.project_id)
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
                                .filter(|m| {
                                    m.role == crate::types::message::MessageRole::User
                                })
                                .map(|m| {
                                    m.content.chars().take(200).collect::<String>()
                                })
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
                                    &user_id,
                                    Some(payload),
                                )
                                .await
                            {
                                warn!("event listener: failed to fire conversation insights: {e}");
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
    ) -> Result<MissionId, EngineError> {
        // Check if one already exists
        let missions = self.store.list_missions(project_id).await?;
        if let Some(existing) = missions.iter().find(|m| is_self_improvement_mission(m)) {
            debug!(mission_id = %existing.id, "self-improvement mission already exists");
            // Make sure it's in the active list
            let mut active = self.active.write().await;
            if !active.contains(&existing.id) {
                active.push(existing.id);
            }
            return Ok(existing.id);
        }

        // Create the self-improvement mission
        let mut mission = Mission::new(
            project_id,
            "self-improvement",
            SELF_IMPROVEMENT_GOAL,
            MissionCadence::OnSystemEvent {
                source: "engine".into(),
                event_type: "thread_completed_with_issues".into(),
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
        let docs = self.store.list_memory_docs(project_id).await?;
        let has_patterns = docs.iter().any(|d| {
            d.title == FIX_PATTERN_DB_TITLE && d.tags.contains(&FIX_PATTERN_DB_TAG.to_string())
        });
        if !has_patterns {
            use crate::types::memory::{DocType, MemoryDoc};
            let pattern_doc = MemoryDoc::new(
                project_id,
                DocType::Playbook,
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
    /// Creates (if missing) the self-improvement, playbook extraction, and
    /// conversation insights missions. This is the preferred entry point —
    /// call once at project bootstrap.
    pub async fn ensure_learning_missions(
        &self,
        project_id: ProjectId,
    ) -> Result<(), EngineError> {
        // 1. Error diagnosis (self-improvement) — existing
        self.ensure_self_improvement_mission(project_id).await?;

        // 2. Skill extraction (formerly playbook extraction)
        self.ensure_mission_by_metadata(
            project_id,
            "skill_extraction",
            "skill-extraction",
            SKILL_EXTRACTION_GOAL,
            MissionCadence::OnSystemEvent {
                source: "engine".into(),
                event_type: "thread_completed_with_learnings".into(),
            },
            "Extract reusable skills from successful multi-step threads",
            3, // max 3/day
        )
        .await?;

        // 3. Conversation insights
        self.ensure_mission_by_metadata(
            project_id,
            "conversation_insights",
            "conversation-insights",
            CONVERSATION_INSIGHTS_GOAL,
            MissionCadence::OnSystemEvent {
                source: "engine".into(),
                event_type: "conversation_insights_due".into(),
            },
            "Extract user preferences, domain knowledge, and workflow patterns from conversations",
            2, // max 2/day
        )
        .await?;

        Ok(())
    }

    /// Ensure a mission with a specific metadata tag exists, creating it if not.
    #[allow(clippy::too_many_arguments)]
    async fn ensure_mission_by_metadata(
        &self,
        project_id: ProjectId,
        metadata_key: &str,
        name: &str,
        goal: &str,
        cadence: MissionCadence,
        success_criteria: &str,
        max_per_day: u32,
    ) -> Result<MissionId, EngineError> {
        let missions = self.store.list_missions(project_id).await?;
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

        let mut mission = Mission::new(project_id, name, goal, cadence);
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
    pub async fn tick(&self, user_id: &str) -> Result<Vec<ThreadId>, EngineError> {
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

            if should_fire && let Some(tid) = self.fire_mission(mid, user_id, None).await? {
                spawned.push(tid);
            }
        }

        Ok(spawned)
    }

    fn spawn_mission_outcome_watcher(&self, mission_id: MissionId, thread_id: ThreadId) {
        let tm = Arc::clone(&self.thread_manager);
        let store = Arc::clone(&self.store);
        tokio::spawn(async move {
            match tm.join_thread(thread_id).await {
                Ok(outcome) => {
                    if let Err(e) =
                        process_mission_outcome(&store, mission_id, thread_id, &outcome).await
                    {
                        warn!(mission_id = %mission_id, "failed to process outcome: {e}");
                    }
                }
                Err(e) => {
                    warn!(mission_id = %mission_id, "thread join failed: {e}");
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
fn build_meta_prompt(
    mission: &Mission,
    project_docs: &[MemoryDoc],
    trigger_payload: &Option<serde_json::Value>,
) -> String {
    let mut parts = Vec::new();

    parts.push(format!(
        "# Mission: {}\n\nGoal: {}",
        mission.name, mission.goal
    ));

    if let Some(criteria) = &mission.success_criteria {
        parts.push(format!("Success criteria: {criteria}"));
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
async fn process_mission_outcome(
    store: &Arc<dyn Store>,
    mission_id: MissionId,
    _thread_id: ThreadId,
    outcome: &ThreadOutcome,
) -> Result<(), EngineError> {
    let mut mission = match store.load_mission(mission_id).await? {
        Some(m) => m,
        None => return Ok(()),
    };

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

            // Record approach
            let accomplishment: String = text.chars().take(200).collect();
            mission.approach_history.push(accomplishment);

            // If this is a self-improvement mission, process structured output
            if is_self_improvement_mission(&mission)
                && let Err(e) = process_self_improvement_output(store, &mission, text).await
            {
                warn!(
                    mission_id = %mission_id,
                    "failed to process self-improvement output: {e}"
                );
            }
        }
        ThreadOutcome::Completed { response: None } => {}
        ThreadOutcome::Failed { error } => {
            mission.approach_history.push(format!("FAILED: {error}"));
        }
        ThreadOutcome::MaxIterations => {
            mission
                .approach_history
                .push("Hit max iterations without completing".into());
        }
        _ => {}
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

    // Process prompt additions
    if let Some(additions) = json_val.get("prompt_additions").and_then(|v| v.as_array())
        && !additions.is_empty()
    {
        let new_rules: Vec<String> = additions
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        if !new_rules.is_empty() {
            // Load or create the prompt overlay doc
            let docs = store.list_memory_docs(project_id).await?;
            let existing = docs.iter().find(|d| {
                d.title == PREAMBLE_OVERLAY_TITLE
                    && d.tags.contains(&PROMPT_OVERLAY_TAG.to_string())
            });

            let mut overlay = if let Some(doc) = existing {
                doc.clone()
            } else {
                MemoryDoc::new(project_id, DocType::Note, PREAMBLE_OVERLAY_TITLE, "")
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
        let docs = store.list_memory_docs(project_id).await?;
        let existing = docs.iter().find(|d| {
            d.title == FIX_PATTERN_DB_TITLE && d.tags.contains(&FIX_PATTERN_DB_TAG.to_string())
        });

        let mut pattern_doc = if let Some(doc) = existing {
            doc.clone()
        } else {
            MemoryDoc::new(
                project_id,
                DocType::Playbook,
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
const SELF_IMPROVEMENT_GOAL: &str = "\
You are a self-improvement agent for the IronClaw engine. You receive trigger \
payloads containing execution trace issues from completed threads. Your job is \
to diagnose root causes and apply fixes so the same issue doesn't recur.

## What you have access to

- `state[\"trigger_payload\"]` — JSON with `issues` (list of {severity, category, description, step}), \
  `error_messages` (actual error text from failed actions), `goal` (what the thread was trying to do), \
  and `source_thread_id`.
- All tools: shell, read_file, write_file, apply_patch, web_search, memory_write, etc.
- The codebase at the current working directory.
- The fix pattern database in prior knowledge (if loaded).

## The experiment loop

For each issue in the trigger payload:

1. **Diagnose**: Read the error messages and issue descriptions. Classify the root cause:
   - PROMPT: The LLM made a mistake because the system prompt is missing a rule \
     (wrong tool name, bad API usage, ignoring tool results)
   - CONFIG: A default value is wrong (truncation length, iteration limit, timeout)
   - CODE: There is a bug in the engine or bridge code (crash, type error, missing conversion)

2. **Check the fix pattern database** in prior knowledge. Has this pattern been seen before? \
   If yes, apply the known strategy. If no, proceed to step 3.

3. **Apply the fix** based on the level:

   Level 1 (PROMPT — low risk, apply directly):
   - Read the current prompt overlay: `memory_search(\"prompt:codeact_preamble\")`
   - Write an updated overlay with a new rule appended
   - Use `memory_write` with title=\"prompt:codeact_preamble\" and tags=[\"prompt_overlay\"]
   - The rule should be specific and actionable (e.g. \"Never call web_fetch — use http() instead\")

   Level 2 (CONFIG — medium risk):
   - Use `read_file` to find the relevant constant or default
   - Use `shell` to create a git branch: `git checkout -b self-improve/issue-description`
   - Apply the change with `apply_patch` or `write_file`
   - Run tests: `cargo test -p ironclaw_engine`
   - If tests pass, commit. If not, revert: `git checkout main`

   Level 3 (CODE — high risk, just propose):
   - Read the relevant source files
   - Describe the fix needed but DO NOT apply it directly
   - Log it as a recommendation in your FINAL() response

4. **Record what you did** — include in your FINAL() response:
   - What issue you analyzed
   - What level fix you applied (1/2/3)
   - What specific change you made
   - Next focus: what to look for next time

## Important rules

- Be specific. \"Never call web_fetch\" is good. \"Be careful with tool names\" is useless.
- One fix per issue. Don't try to fix everything at once.
- For Level 1 fixes, the rule must be one sentence that can be appended to the prompt.
- If the trigger payload has no actionable issues (only Info severity), skip and call FINAL() immediately.
- NEVER modify test files to make a fix pass.
- NEVER modify security-sensitive code (safety layer, policy engine, leak detection).
- If you can't diagnose the root cause after reading the errors, log it and move on.

## Level 1.5: Orchestrator patches (medium risk, auto-rollback)

The execution loop itself is Python code that you can modify. This is the \
orchestrator — it handles tool dispatch, output formatting, state management, \
and context building. If the bug is in the glue between the LLM and tools \
(wrong output format, bad truncation, missing state), you can patch it directly.

To modify the orchestrator:
1. Read current version: `memory_search(\"orchestrator:main\")`
2. Make your change (keep it minimal — one fix at a time)
3. Save the new version: `memory_write` with title=\"orchestrator:main\", \
   tags=[\"orchestrator_code\"], metadata={\"version\": N+1, \"parent_version\": N}
4. The next thread will use your updated orchestrator

If your change causes 3 consecutive failures, the system auto-rolls back to \
the previous version. So be conservative — test your logic mentally before saving.";

/// Well-known title for the fix pattern database.
pub const FIX_PATTERN_DB_TITLE: &str = "fix_pattern_database";

/// Well-known tag for the fix pattern database.
pub const FIX_PATTERN_DB_TAG: &str = "fix_patterns";

/// The goal for the skill extraction mission (replaces playbook extraction).
const SKILL_EXTRACTION_GOAL: &str = "\
You extract reusable skills from successfully completed multi-step threads.

## Input

`state[\"trigger_payload\"]` contains:
- `source_thread_id` — the thread that completed successfully
- `goal` — what the thread accomplished
- `step_count` — number of execution steps
- `action_count` — number of tool actions executed
- `actions_used` — list of tool names used
- `total_tokens` — tokens consumed

## Output Format

Save as a Skill memory doc via `memory_write(target=\"memory\", content=skill_prompt)` with:
- title: `\"skill:<short-name>\"` (e.g., \"skill:github-issue-triage\")
- doc_type: `\"skill\"`
- metadata JSON:
  ```json
  {
    \"name\": \"<short-name>\",
    \"version\": 1,
    \"description\": \"<one-line description>\",
    \"activation\": {
      \"keywords\": [\"<keyword1>\", \"<keyword2>\"],
      \"patterns\": [\"<optional regex>\"],
      \"tags\": [\"<domain-tag>\"],
      \"exclude_keywords\": [],
      \"max_context_tokens\": <estimated budget, e.g. 1000>
    },
    \"source\": \"extracted\",
    \"trust\": \"trusted\",
    \"code_snippets\": [
      {
        \"name\": \"<function_name>\",
        \"code\": \"def <function_name>(...):\\n    ...\",
        \"description\": \"<what it does>\"
      }
    ],
    \"metrics\": {\"usage_count\": 0, \"success_count\": 0, \"failure_count\": 0},
    \"content_hash\": \"\"
  }
  ```

## Process

1. Search for the source thread's context: `memory_search(query=goal)`
2. Check for existing skills: `memory_search(query=\"skill:\")`
3. If a similar skill exists, update it (increment version) rather than creating a duplicate
4. Extract:
   - Activation keywords from the goal + user messages (be specific, not generic)
   - Step-by-step instructions as the prompt content
   - Python code snippets for CodeAct (reusable functions using exact tool names)
   - Domain tags (e.g., \"github\", \"api\", \"data\")

## Output (FINAL)

Report what you did:
- The skill title and a one-line summary
- Whether it is new or an update to an existing skill
- Next focus: what patterns to watch for

## Rules

- Only extract skills from threads with 3+ distinct tool calls
- Keywords must be specific (not generic words like \"help\", \"do\", \"make\")
- Code snippets must use exact tool function names as they appear in the thread
- If the thread was a trivial query-response, call FINAL(\"No skill needed — simple interaction\") \
  and stop immediately
- One skill per FINAL — do not combine unrelated procedures
";

/// The goal for the conversation insights mission.
const CONVERSATION_INSIGHTS_GOAL: &str = "\
You extract user preferences, patterns, and domain knowledge from a batch of recent \
conversation threads.

## Input

`state[\"trigger_payload\"]` contains:
- `project_id` — the project scope
- `completed_thread_count` — total threads completed in this conversation
- `thread_goals` — list of recent thread goals (what the user asked for)
- `sample_user_messages` — sample of actual user messages (truncated to 200 chars)

## Process

1. Analyze the thread goals and user messages for patterns
2. Search existing insights: `memory_search(query=\"user preferences\")` and \
   `memory_search(query=\"domain knowledge\")`
3. Extract NEW insights not already recorded in memory
4. Write each insight to memory via `memory_write(target=\"memory\", content=insight_text)` \
   with title format \"insight:<category>:<topic>\"

## Categories to look for

- **Preferences**: communication style, format choices, tool preferences
- **Domain**: project names, API patterns, data formats, technology stack
- **Workflow**: recurring task sequences, common follow-up questions
- **Corrections**: things the user corrected or repeated — these signal unmet expectations

## Output (FINAL)

Report:
- Number of new insights extracted (0 is fine)
- Brief list of what was found
- Next focus

## Rules

- Only record actionable, specific insights — not vague observations
- Do not record personal information, only work patterns
- If no meaningful new insights after analysis, call FINAL(\"No new insights — \
  conversation patterns already captured\") immediately
- Merge with existing insight docs rather than creating duplicates
- Max 5 insights per run to keep quality high
";

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
        async fn list_threads(&self, _: ProjectId) -> Result<Vec<Thread>, EngineError> {
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
        async fn list_missions(&self, project_id: ProjectId) -> Result<Vec<Mission>, EngineError> {
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
                "test mission",
                "do the thing",
                MissionCadence::Manual,
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
            .create_mission(project_id, "pausable", "goal", MissionCadence::Manual)
            .await
            .unwrap();

        // Pause
        mgr.pause_mission(id).await.unwrap();
        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(mission.status, MissionStatus::Paused);

        // Resume
        mgr.resume_mission(id).await.unwrap();
        let mission = mgr.get_mission(id).await.unwrap().unwrap();
        assert_eq!(mission.status, MissionStatus::Active);
    }

    #[tokio::test]
    async fn complete_removes_from_active() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(project_id, "completable", "goal", MissionCadence::Manual)
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
                "fireable",
                "build something",
                MissionCadence::Manual,
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
            .create_mission(project_id, "terminal", "goal", MissionCadence::Manual)
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
                "cron mission",
                "periodic goal",
                MissionCadence::Cron {
                    expression: "* * * * *".into(),
                    timezone: None,
                },
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
                "Tech News",
                "Deliver daily tech news briefing",
                MissionCadence::Manual,
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
                "Test Coverage",
                "Increase test coverage to 80%",
                MissionCadence::Manual,
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
                "Coverage Mission",
                "Get to 80% coverage",
                MissionCadence::Manual,
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
                "GitHub Triage",
                "Triage incoming issues",
                MissionCadence::Webhook {
                    path: "github".into(),
                    secret: None,
                },
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
            "self-improve",
            "improve prompts",
            MissionCadence::OnSystemEvent {
                source: "engine".into(),
                event_type: "thread_completed_with_issues".into(),
            },
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
            "webhook handler",
            "handle webhooks",
            MissionCadence::OnSystemEvent {
                source: "github".into(),
                event_type: "push".into(),
            },
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

        mgr.create_mission(project_id, "manual", "goal", MissionCadence::Manual)
            .await
            .unwrap();
        mgr.create_mission(
            project_id,
            "cron",
            "goal",
            MissionCadence::Cron {
                expression: "* * * * *".into(),
                timezone: None,
            },
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
            "self-improve",
            "improve prompts",
            MissionCadence::OnSystemEvent {
                source: "engine".into(),
                event_type: "thread_completed_with_issues".into(),
            },
        );
        mission.metadata = serde_json::json!({"self_improvement": true});
        let id = mission.id;
        store.save_mission(&mission).await.unwrap();

        let response = r#"{"prompt_additions": ["9. Never call web_fetch — use http() instead."], "fix_patterns": [], "level": 1}"#;
        let outcome = ThreadOutcome::Completed {
            response: Some(response.into()),
        };
        process_mission_outcome(&store, id, ThreadId::new(), &outcome)
            .await
            .unwrap();

        // Verify prompt overlay was saved
        let docs = store.list_memory_docs(project_id).await.unwrap();
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

        let docs = store.list_memory_docs(project_id).await.unwrap();
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

        let mission = Mission::new(project_id, "regular", "do stuff", MissionCadence::Manual);
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

        let docs = store.list_memory_docs(project_id).await.unwrap();
        assert!(docs.is_empty(), "non-SI mission should not create overlay");
    }

    #[tokio::test]
    async fn ensure_self_improvement_mission_creates_on_first_call() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager(Arc::clone(&store) as Arc<dyn Store>);
        let project_id = ProjectId::new();

        let id = mgr
            .ensure_self_improvement_mission(project_id)
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

        // Fix pattern database should be seeded
        let docs = store.list_memory_docs(project_id).await.unwrap();
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
            .ensure_self_improvement_mission(project_id)
            .await
            .unwrap();
        let id2 = mgr
            .ensure_self_improvement_mission(project_id)
            .await
            .unwrap();

        assert_eq!(id1, id2, "should return the same mission ID");

        // Should only have one mission
        let missions = store.list_missions(project_id).await.unwrap();
        assert_eq!(missions.len(), 1);
    }

    #[tokio::test]
    async fn daily_budget_enforced() {
        let store = Arc::new(TestStore::new());
        let mgr = make_mission_manager_with_response(Arc::clone(&store) as Arc<dyn Store>, "done");
        let project_id = ProjectId::new();

        let id = mgr
            .create_mission(project_id, "budget test", "goal", MissionCadence::Manual)
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
}
