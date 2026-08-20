//! Docker label identity plus bounded process-local lifecycle bookkeeping.
//!
//! User containers carry immutable tenant, user, image, and posture labels so
//! a new IronClaw process can adopt them safely. Mutable active-exec counts,
//! activity timestamps, and per-user lifecycle gates remain in
//! [`SandboxActivityRegistry`]. The background-job registry stays separate
//! because those jobs track process metadata rather than container lifecycle.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use bollard::models::ContainerSummary;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    ids::{TenantId, UserId},
    process::RuntimeProcessError,
};

use crate::sandbox_process::user_key::RebornSandboxUserKey;

pub(crate) const USER_CONTAINER_LABEL_PREFIX: &str = "ironclaw";
pub(crate) const USER_CONTAINER_LABEL_TENANT: &str = "ironclaw.tenant";
pub(crate) const USER_CONTAINER_LABEL_USER: &str = "ironclaw.user";
pub(crate) const USER_CONTAINER_LABEL_IMAGE: &str = "ironclaw.image";
pub(crate) const USER_CONTAINER_LABEL_SECURITY_POSTURE: &str = "ironclaw.security_posture";

pub(crate) fn label_tenant(prefix: &str) -> String {
    format!("{prefix}.tenant")
}
pub(crate) fn label_user(prefix: &str) -> String {
    format!("{prefix}.user")
}
pub(crate) fn label_image(prefix: &str) -> String {
    format!("{prefix}.image")
}
/// Creation timestamp used by both launch compatibility and attribution.
pub(crate) fn label_created_at(prefix: &str) -> String {
    format!("{prefix}.created_at")
}

/// Label for the deterministic security and mount posture used at creation.
pub(crate) fn label_security_posture(prefix: &str) -> String {
    format!("{prefix}.security_posture")
}

// Retained for attribution callers that still need only per-user labels.
#[allow(dead_code)]
pub(crate) fn build_user_container_labels(
    prefix: &str,
    tenant_id: &TenantId,
    user_id: &UserId,
    security_posture_stamp: &str,
) -> HashMap<String, String> {
    HashMap::from([
        (label_tenant(prefix), tenant_id.as_str().to_string()),
        (label_user(prefix), user_id.as_str().to_string()),
        (label_created_at(prefix), Utc::now().to_rfc3339()),
        (
            label_security_posture(prefix),
            security_posture_stamp.to_string(),
        ),
    ])
}
pub(crate) fn build_user_container_launch_labels(
    prefix: &str,
    tenant_id: &TenantId,
    user_id: &UserId,
    image: &str,
    security_posture_stamp: &str,
) -> HashMap<String, String> {
    let mut labels =
        build_user_container_labels(prefix, tenant_id, user_id, security_posture_stamp);
    labels.insert(label_image(prefix), image.to_string());
    labels
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistingContainerDecision {
    ReuseRunning,
    StartStopped,
    Recreate,
}

pub(crate) fn existing_container_decision(
    labels: Option<&HashMap<String, String>>,
    running: bool,
    expected: &HashMap<String, String>,
) -> ExistingContainerDecision {
    let compatible = labels.is_some_and(|labels| {
        expected
            .iter()
            .filter(|(key, _)| !key.ends_with(".created_at"))
            .all(|(key, value)| labels.get(key) == Some(value))
    });
    if !compatible {
        ExistingContainerDecision::Recreate
    } else if running {
        ExistingContainerDecision::ReuseRunning
    } else {
        ExistingContainerDecision::StartStopped
    }
}

// Retained for per-user attribution lookup.
#[allow(dead_code)]
pub(crate) fn user_container_label_filter(
    prefix: &str,
    tenant_id: &TenantId,
    user_id: &UserId,
) -> HashMap<String, Vec<String>> {
    HashMap::from([(
        "label".to_string(),
        vec![
            format!("{}={}", label_tenant(prefix), tenant_id.as_str()),
            format!("{}={}", label_user(prefix), user_id.as_str()),
        ],
    )])
}

// Retained for older per-user recycle candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct UserContainerCandidate {
    pub(crate) container_id: String,
    pub(crate) created_at: DateTime<Utc>,
}

#[allow(dead_code)]
impl UserContainerCandidate {
    pub(crate) fn from_summary(container: &ContainerSummary, label_prefix: &str) -> Option<Self> {
        let container_id = container.id.clone()?;
        let labels = container.labels.as_ref()?;
        // silent-ok: a missing or unparseable created_at label means this
        // container is not a valid recycle candidate; the caller already
        // treats a rejected candidate as "not ours" via the `?` short
        // circuit, so the parse error itself is not needed to make that
        // decision.
        let created_at = labels
            .get(&label_created_at(label_prefix))
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))?;
        Some(Self {
            container_id,
            created_at,
        })
    }
}

/// Per-user execution activity and lifecycle gates. Each gate serializes shell
/// commands for one tenant and user; different users have different gates.
/// Mutable activity is process-local; immutable identity and compatibility
/// remain in Docker labels.
#[derive(Debug, Default)]
pub struct SandboxActivityRegistry {
    state: Mutex<HashMap<RebornSandboxUserKey, ActivityEntry>>,
}

#[derive(Debug)]
struct ActivityEntry {
    last_activity: Instant,
    active_execs: usize,
    gate: Arc<tokio::sync::Mutex<()>>,
    expected_labels: Option<HashMap<String, String>>,
    recycle_required: bool,
}

const MAX_TRACKED_USERS: usize = 4096;

impl SandboxActivityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RebornSandboxUserKey, ActivityEntry>> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(crate) fn begin(
        self: &Arc<Self>,
        key: &RebornSandboxUserKey,
    ) -> Result<SandboxActivityGuard, RuntimeProcessError> {
        let mut state = self.lock();
        if !state.contains_key(key) && state.len() >= MAX_TRACKED_USERS {
            let oldest_inactive = state
                .iter()
                .filter(|(_, entry)| {
                    entry.active_execs == 0
                        && !entry.recycle_required
                        && Arc::strong_count(&entry.gate) == 1
                        && entry.expected_labels.is_none()
                })
                .min_by_key(|(_, entry)| entry.last_activity)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest_inactive {
                state.remove(&oldest);
            } else {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "sandbox user activity registry is at capacity".to_string(),
                ));
            }
        }
        let entry = state.entry(key.clone()).or_insert_with(|| ActivityEntry {
            last_activity: Instant::now(),
            active_execs: 0,
            gate: Arc::new(tokio::sync::Mutex::new(())),
            expected_labels: None,
            recycle_required: false,
        });
        entry.active_execs = entry.active_execs.saturating_add(1);
        entry.last_activity = Instant::now();
        Ok(SandboxActivityGuard {
            registry: Arc::clone(self),
            key: key.clone(),
        })
    }

    pub(crate) fn gate(&self, key: &RebornSandboxUserKey) -> Option<Arc<tokio::sync::Mutex<()>>> {
        self.lock().get(key).map(|entry| Arc::clone(&entry.gate))
    }

    pub(crate) fn set_expected_labels(
        &self,
        key: &RebornSandboxUserKey,
        labels: HashMap<String, String>,
    ) {
        if let Some(entry) = self.lock().get_mut(key) {
            entry.expected_labels = Some(labels);
        }
    }

    pub(crate) fn mark_recycle_required(&self, key: &RebornSandboxUserKey) {
        if let Some(entry) = self.lock().get_mut(key) {
            entry.recycle_required = true;
        }
    }

    pub(crate) fn clear_recycle_required(&self, key: &RebornSandboxUserKey) {
        if let Some(entry) = self.lock().get_mut(key) {
            entry.recycle_required = false;
        }
    }

    pub(crate) fn recycle_required(&self, key: &RebornSandboxUserKey) -> bool {
        self.lock()
            .get(key)
            .is_some_and(|entry| entry.recycle_required)
    }

    pub(crate) fn register_discovered_container(
        &self,
        key: RebornSandboxUserKey,
        labels: HashMap<String, String>,
    ) -> Result<(), RuntimeProcessError> {
        let mut state = self.lock();
        if let Some(entry) = state.get_mut(&key) {
            entry.expected_labels = Some(labels);
            return Ok(());
        }
        if state.len() >= MAX_TRACKED_USERS {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox user activity registry is at capacity".to_string(),
            ));
        }
        state.insert(
            key,
            ActivityEntry {
                last_activity: Instant::now(),
                active_execs: 0,
                gate: Arc::new(tokio::sync::Mutex::new(())),
                expected_labels: Some(labels),
                recycle_required: false,
            },
        );
        Ok(())
    }

    pub(crate) fn sweep_candidates(
        &self,
        now: Instant,
        idle_timeout: Duration,
    ) -> Vec<(RebornSandboxUserKey, HashMap<String, String>)> {
        self.lock()
            .iter()
            .filter(|(_, entry)| {
                entry.active_execs == 0
                    && !entry.recycle_required
                    && now.saturating_duration_since(entry.last_activity) >= idle_timeout
            })
            .filter_map(|(key, entry)| {
                entry
                    .expected_labels
                    .as_ref()
                    .map(|labels| (key.clone(), labels.clone()))
            })
            .collect()
    }

    pub(crate) fn sweep_eligible(
        &self,
        key: &RebornSandboxUserKey,
        now: Instant,
        idle_timeout: Duration,
    ) -> bool {
        self.lock().get(key).is_some_and(|entry| {
            entry.active_execs == 0
                && !entry.recycle_required
                && now.saturating_duration_since(entry.last_activity) >= idle_timeout
        })
    }

    pub(crate) fn forget_if_inactive(&self, key: &RebornSandboxUserKey) {
        let mut state = self.lock();
        if state
            .get(key)
            .is_some_and(|entry| entry.active_execs == 0 && !entry.recycle_required)
        {
            state.remove(key);
        }
    }
}

pub(crate) struct SandboxActivityGuard {
    registry: Arc<SandboxActivityRegistry>,
    key: RebornSandboxUserKey,
}

impl Drop for SandboxActivityGuard {
    fn drop(&mut self) {
        if let Some(entry) = self.registry.lock().get_mut(&self.key) {
            entry.active_execs = entry.active_execs.saturating_sub(1);
            entry.last_activity = Instant::now();
        }
    }
}

/// A single tracked background (`background: true`) shell launch, kept
/// per-user so the foreground command path can render a "still-live
/// background processes" footer. A named struct rather than a `(u32,
/// String)` tuple — `jobs_for` return values flow into formatting code
/// where a bare tuple's field order is not self-documenting.
// consumed by exec_transport's background-job tracking, not in this PR
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct BackgroundJob {
    pub(crate) pid: u32,
    pub(crate) command_preview: String,
}

/// Push-based in-memory map of per-user background job launches, keyed on
/// [`RebornSandboxUserKey`]. This remains separate from
/// [`SandboxActivityRegistry`].
///
/// No production caller in this PR — see [`BackgroundJob`].
///
/// **Known tradeoff, not fixed here:** nothing bounds how many jobs
/// accumulate per user before `drop_dead` prunes them, and `jobs_for` clones
/// the whole retained `Vec` on every call. The right bound (job-count cap?
/// command-preview truncation?) depends on exec_transport's launch rate and
/// the reaper's `drop_dead` cadence, neither of which exist yet — bounding
/// blind, before that caller is wired, risks guessing a cap that's wrong in
/// either direction. `drop_dead` is already the exact pruning seam the
/// future reaper hookup calls; left as a follow-up for that PR rather than
/// spec'd speculatively here.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub(crate) struct BackgroundJobRegistry {
    jobs: Mutex<HashMap<RebornSandboxUserKey, Vec<BackgroundJob>>>,
}

#[allow(dead_code)]
impl BackgroundJobRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RebornSandboxUserKey, Vec<BackgroundJob>>> {
        self.jobs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(crate) fn record(&self, key: &RebornSandboxUserKey, pid: u32, command_preview: String) {
        self.lock()
            .entry(key.clone())
            .or_default()
            .push(BackgroundJob {
                pid,
                command_preview,
            });
    }

    pub(crate) fn jobs_for(&self, key: &RebornSandboxUserKey) -> Vec<BackgroundJob> {
        self.lock().get(key).cloned().unwrap_or_default()
    }

    pub(crate) fn drop_dead(&self, key: &RebornSandboxUserKey, alive_pids: &[u32]) {
        // Build the membership set once per call rather than doing an O(A)
        // linear scan of `alive_pids` per retained job (O(J x A) overall) —
        // with many tracked background jobs this reaper-driven prune runs
        // repeatedly, so constant-time membership checks matter here.
        let alive: std::collections::HashSet<u32> = alive_pids.iter().copied().collect();
        if let Some(jobs) = self.lock().get_mut(key) {
            jobs.retain(|job| alive.contains(&job.pid));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        ids::{TenantId, ThreadId, UserId},
        resource::ResourceScope,
    };
    fn active_execs(registry: &SandboxActivityRegistry, key: &RebornSandboxUserKey) -> usize {
        registry
            .lock()
            .get(key)
            .map_or(0, |entry| entry.active_execs)
    }

    fn set_last_activity(
        registry: &SandboxActivityRegistry,
        key: &RebornSandboxUserKey,
        activity: Instant,
    ) {
        if let Some(entry) = registry.lock().get_mut(key) {
            entry.last_activity = activity;
        }
    }

    #[test]
    fn label_filter_targets_tenant_and_user_labels_only() {
        let tenant = TenantId::new("tenant-a").unwrap();
        let user = UserId::new("user-a").unwrap();

        let filter = user_container_label_filter("ironclaw", &tenant, &user);

        assert_eq!(
            filter.get("label").unwrap(),
            &vec![
                "ironclaw.tenant=tenant-a".to_string(),
                "ironclaw.user=user-a".to_string(),
            ]
        );
    }
    #[test]
    fn compatibility_requires_every_user_identity_and_posture_label() {
        let tenant = TenantId::new("tenant-a").unwrap();
        let user = UserId::new("user-a").unwrap();
        let expected =
            build_user_container_launch_labels("ironclaw", &tenant, &user, "image:v1", "p1");

        assert_eq!(
            existing_container_decision(Some(&expected), true, &expected),
            ExistingContainerDecision::ReuseRunning
        );
        assert_eq!(
            existing_container_decision(Some(&expected), false, &expected),
            ExistingContainerDecision::StartStopped
        );
        assert!(!expected.contains_key("ironclaw.thread"));

        for key in [
            label_tenant("ironclaw"),
            label_user("ironclaw"),
            label_image("ironclaw"),
            label_security_posture("ironclaw"),
        ] {
            let mut mismatched = expected.clone();
            mismatched.insert(key, "different".to_string());
            assert_eq!(
                existing_container_decision(Some(&mismatched), true, &expected),
                ExistingContainerDecision::Recreate
            );
        }
        assert_eq!(
            existing_container_decision(None, true, &expected),
            ExistingContainerDecision::Recreate
        );
    }

    #[test]
    fn candidate_parses_created_at_and_ignores_missing_labels() {
        let tenant = TenantId::new("tenant-a").unwrap();
        let user = UserId::new("user-a").unwrap();
        let labels = build_user_container_labels("ironclaw", &tenant, &user, "stamp-abc");
        let container = ContainerSummary {
            id: Some("abc123".to_string()),
            labels: Some(labels),
            ..Default::default()
        };

        let candidate = UserContainerCandidate::from_summary(&container, "ironclaw")
            .expect("round-tripped labels must parse");

        assert_eq!(candidate.container_id, "abc123");

        let missing = ContainerSummary {
            id: Some("no-labels".to_string()),
            labels: None,
            ..Default::default()
        };
        assert!(UserContainerCandidate::from_summary(&missing, "ironclaw").is_none());
    }

    #[test]
    fn candidate_parsing_rejects_a_missing_container_id() {
        let tenant = TenantId::new("tenant-a").unwrap();
        let user = UserId::new("user-a").unwrap();
        let labels = build_user_container_labels("ironclaw", &tenant, &user, "stamp-abc");
        let container = ContainerSummary {
            id: None,
            labels: Some(labels),
            ..Default::default()
        };

        assert!(UserContainerCandidate::from_summary(&container, "ironclaw").is_none());
    }

    #[test]
    fn candidate_parsing_rejects_a_malformed_created_at_label() {
        let mut labels = HashMap::new();
        labels.insert(label_created_at("ironclaw"), "not-a-timestamp".to_string());
        let container = ContainerSummary {
            id: Some("abc123".to_string()),
            labels: Some(labels),
            ..Default::default()
        };

        assert!(UserContainerCandidate::from_summary(&container, "ironclaw").is_none());
    }

    fn test_scope(tenant: &str, user: &str, thread: Option<&str>) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: thread.map(|value| ThreadId::new(value).unwrap()),
            invocation_id: ironclaw_host_api::ids::InvocationId::new(),
        }
    }

    fn test_key(tenant: &str, user: &str) -> RebornSandboxUserKey {
        RebornSandboxUserKey::from_scope(&test_scope(tenant, user, None))
    }

    #[test]
    fn background_job_registry_records_and_returns_jobs_for_a_user() {
        let registry = BackgroundJobRegistry::new();
        let key = test_key("t", "u");

        assert!(registry.jobs_for(&key).is_empty());

        registry.record(&key, 111, "sleep 60".to_string());
        registry.record(&key, 222, "tail -f log".to_string());

        let jobs = registry.jobs_for(&key);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].pid, 111);
        assert_eq!(jobs[0].command_preview, "sleep 60");
        assert_eq!(jobs[1].pid, 222);
    }

    #[test]
    fn background_job_registry_isolates_jobs_by_user_key() {
        let registry = BackgroundJobRegistry::new();
        let a = test_key("t", "user-a");
        let b = test_key("t", "user-b");

        registry.record(&a, 111, "sleep 60".to_string());

        assert_eq!(registry.jobs_for(&a).len(), 1);
        assert!(registry.jobs_for(&b).is_empty());
    }

    #[test]
    fn background_job_registry_drop_dead_retains_only_alive_pids() {
        let registry = BackgroundJobRegistry::new();
        let key = test_key("t", "u");
        registry.record(&key, 111, "sleep 60".to_string());
        registry.record(&key, 222, "tail -f log".to_string());
        registry.record(&key, 333, "sleep 10".to_string());

        registry.drop_dead(&key, &[222]);

        let jobs = registry.jobs_for(&key);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].pid, 222);
    }

    #[test]
    fn background_job_registry_drop_dead_on_unknown_key_is_a_no_op() {
        let registry = BackgroundJobRegistry::new();
        let key = test_key("t", "u");

        // Must not panic when the key was never recorded.
        registry.drop_dead(&key, &[111]);

        assert!(registry.jobs_for(&key).is_empty());
    }

    #[test]
    fn background_job_registry_drop_dead_with_empty_alive_pids_removes_all_jobs() {
        let registry = BackgroundJobRegistry::new();
        let key = test_key("t", "u");
        registry.record(&key, 111, "sleep 60".to_string());
        registry.record(&key, 222, "tail -f log".to_string());

        registry.drop_dead(&key, &[]);

        assert!(registry.jobs_for(&key).is_empty());
    }

    #[test]
    fn background_job_registry_survives_concurrent_record_read_and_drop_dead() {
        // `BackgroundJobRegistry` owns its own `Mutex`, separate from
        // `SandboxActivityRegistry`'s — this drives real concurrent access
        // from multiple OS threads across `record`/`jobs_for`/`drop_dead` so
        // that lock is exercised the same way the activity registry's is
        // above. The assertion is simply that it completes without
        // panicking/deadlocking, and that state from a still-recording
        // thread is observable afterwards.
        use std::sync::Arc;
        use std::thread;

        let registry = Arc::new(BackgroundJobRegistry::new());
        let keys: Vec<RebornSandboxUserKey> = (0..8)
            .map(|index| test_key("t", &format!("user-{index}")))
            .collect();

        let mut handles = Vec::new();
        for (index, key) in keys.clone().into_iter().enumerate() {
            let registry = Arc::clone(&registry);
            handles.push(thread::spawn(move || {
                for iteration in 0..200 {
                    let pid = (index * 1000 + iteration) as u32;
                    registry.record(&key, pid, format!("job-{pid}"));
                    let _ = registry.jobs_for(&key);
                    registry.drop_dead(&key, &[pid]);
                }
            }));
        }
        // A concurrent record/drop_dead on a disjoint key, so one thread's
        // record/jobs_for races with another thread's drop_dead on the same
        // underlying map.
        let prune_registry = Arc::clone(&registry);
        let prune_key = test_key("t", "user-prune-target");
        prune_registry.record(&prune_key, 1, "seed".to_string());
        handles.push(thread::spawn(move || {
            for iteration in 0..200 {
                prune_registry.record(&prune_key, iteration as u32, "job".to_string());
                prune_registry.drop_dead(&prune_key, &[]);
            }
        }));

        for handle in handles {
            handle
                .join()
                .expect("registry access thread must not panic");
        }

        for key in &keys {
            // Every record/drop_dead pair above leaves exactly the just-
            // recorded pid alive, so the map must still contain that entry.
            assert_eq!(registry.jobs_for(key).len(), 1);
        }
    }

    #[test]
    fn lifecycle_gate_serializes_one_user_but_not_different_users() {
        let registry = Arc::new(SandboxActivityRegistry::new());
        let first = RebornSandboxUserKey::from_scope(&test_scope("t", "u", Some("thread-one")));
        let second = RebornSandboxUserKey::from_scope(&test_scope("t", "u", Some("thread-two")));
        let without_thread = RebornSandboxUserKey::from_scope(&test_scope("t", "u", None));
        let other_user = RebornSandboxUserKey::from_scope(&test_scope("t", "other", None));

        let first_guard = registry.begin(&first).unwrap();
        let second_guard = registry.begin(&second).unwrap();
        let other_user_guard = registry.begin(&other_user).unwrap();

        assert_eq!(first, second);
        assert_eq!(first, without_thread);
        let first_gate = registry.gate(&first).unwrap();
        let same_user_gate = registry.gate(&without_thread).unwrap();
        let other_user_gate = registry.gate(&other_user).unwrap();
        assert!(Arc::ptr_eq(&first_gate, &same_user_gate));
        assert!(!Arc::ptr_eq(&first_gate, &other_user_gate));
        let _first_lock = first_gate.try_lock().unwrap();
        assert!(same_user_gate.try_lock().is_err());
        assert!(other_user_gate.try_lock().is_ok());
        assert_eq!(active_execs(&registry, &without_thread), 2);

        drop(first_guard);
        drop(second_guard);
        drop(other_user_guard);
        assert_eq!(active_execs(&registry, &first), 0);
    }

    #[test]
    fn activity_guard_tracks_active_exec_and_updates_last_activity_on_drop() {
        let registry = Arc::new(SandboxActivityRegistry::new());
        let key = test_key("t", "u");
        let guard = registry.begin(&key).unwrap();
        registry.set_expected_labels(
            &key,
            HashMap::from([("identity".to_string(), "v".to_string())]),
        );

        assert_eq!(active_execs(&registry, &key), 1);
        assert!(
            registry
                .sweep_candidates(Instant::now(), Duration::ZERO)
                .is_empty()
        );

        drop(guard);
        assert_eq!(active_execs(&registry, &key), 0);
        let candidates = registry.sweep_candidates(Instant::now(), Duration::ZERO);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, key);
    }

    #[test]
    fn sweep_eligibility_requires_zero_active_execs_and_elapsed_idle_timeout() {
        let registry = Arc::new(SandboxActivityRegistry::new());
        let key = test_key("t", "u");
        let guard = registry.begin(&key).unwrap();
        set_last_activity(&registry, &key, Instant::now() - Duration::from_secs(60));

        assert!(!registry.sweep_eligible(&key, Instant::now(), Duration::from_secs(30)));
        drop(guard);
        assert!(!registry.sweep_eligible(&key, Instant::now(), Duration::from_secs(30)));

        set_last_activity(&registry, &key, Instant::now() - Duration::from_secs(60));
        assert!(registry.sweep_eligible(&key, Instant::now(), Duration::from_secs(30)));
    }

    #[test]
    fn inactive_entries_can_be_pruned_but_active_entries_cannot() {
        let registry = Arc::new(SandboxActivityRegistry::new());
        let key = test_key("t", "u");
        let guard = registry.begin(&key).unwrap();

        registry.forget_if_inactive(&key);
        assert_eq!(active_execs(&registry, &key), 1);

        drop(guard);
        registry.forget_if_inactive(&key);
        assert!(registry.gate(&key).is_none());
    }
    #[test]
    fn registry_capacity_evicts_only_container_free_entries() {
        let registry = Arc::new(SandboxActivityRegistry::new());
        for index in 0..MAX_TRACKED_USERS {
            let key = test_key("tenant", &format!("free-{index}"));
            drop(registry.begin(&key).unwrap());
        }

        let extra = test_key("tenant", "extra");
        drop(
            registry
                .begin(&extra)
                .expect("container-free entry is evictable"),
        );
        assert!(registry.gate(&extra).is_some());
        assert_eq!(registry.lock().len(), MAX_TRACKED_USERS);
    }

    #[test]
    fn registry_capacity_never_evicts_container_backed_entries() {
        let registry = Arc::new(SandboxActivityRegistry::new());
        for index in 0..MAX_TRACKED_USERS {
            let key = test_key("tenant", &format!("container-{index}"));
            drop(registry.begin(&key).unwrap());
            registry.set_expected_labels(
                &key,
                HashMap::from([("container".to_string(), index.to_string())]),
            );
        }
        let result = registry.begin(&test_key("tenant", "overflow"));
        let Err(error) = result else {
            panic!("container-backed entries must not be orphaned");
        };
        assert_eq!(
            error,
            RuntimeProcessError::ExecutionFailed(
                "sandbox user activity registry is at capacity".to_string()
            )
        );
        assert_eq!(registry.lock().len(), MAX_TRACKED_USERS);
    }
}
