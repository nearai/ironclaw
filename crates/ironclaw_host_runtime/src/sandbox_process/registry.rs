//! Labels-as-identity container registry for the persistent per-user
//! sandbox model. Three responsibilities:
//!
//! 1. Docker label construction/parsing for `{tenant, user, created_at}`
//!    identity — crash-safe (survives daemon restart), no DB.
//! 2. A push-based in-memory last-activity map ([`SandboxActivityRegistry`]).
//!    The exec transport calls [`SandboxActivityRegistry::touch`] after
//!    every successful command; the reaper only ever reads via
//!    `idle_for`/`last_activity` — it never inspects container stats to
//!    infer activity. Labels are immutable post-create and NEVER carry this
//!    mutable state.
//! 3. A push-based in-memory per-user background-job map
//!    ([`BackgroundJobRegistry`]), keyed the same way, so the foreground
//!    command path can render a "still-live background processes" footer.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use bollard::models::ContainerSummary;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{TenantId, UserId};

use crate::sandbox_process::user_key::RebornSandboxUserKey;

pub(crate) fn label_tenant(prefix: &str) -> String {
    format!("{prefix}.tenant")
}
pub(crate) fn label_user(prefix: &str) -> String {
    format!("{prefix}.user")
}
#[allow(dead_code)] // consumed by exec_transport's container-create path, not in this PR
pub(crate) fn label_created_at(prefix: &str) -> String {
    format!("{prefix}.created_at")
}

/// Stamps the container with the security-posture generation
/// [`super::exec_transport::security_posture_stamp`] computed at create
/// time — the container-side analogue of `verify_existing_egress_network_
/// posture`'s network check. `ensure_container` reads this label back on
/// every subsequent lookup and recycles the container the moment the stamp
/// no longer matches what the running code would create today, so a
/// hardening change (e.g. W1's non-root PID 1) reaches existing containers
/// on their next use instead of waiting up to the reaper's 7-day forced
/// recycle.
#[allow(dead_code)] // consumed by exec_transport's container-create path, not in this PR
pub(crate) fn label_security_posture(prefix: &str) -> String {
    format!("{prefix}.security_posture")
}

// Only called from `#[cfg(test)]` code in this PR (this file's own tests and
// attribution.rs's `real_docker_resolves_a_live_container_on_the_egress_network`)
// — dead_code still fires on a non-test build because the real caller
// (exec_transport's container-create path) is not in this PR.
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

// consumed by exec_transport's container-lookup path, not in this PR
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

// consumed by exec_transport's container-recycle path, not in this PR
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

/// Push-based in-memory map of per-user last-activity timestamps, keyed on
/// [`RebornSandboxUserKey`]. Cross-crate consumers (e.g. the reborn runtime
/// composition wiring that owns the reaper loop) need to construct and pass
/// this registry, so it is `pub` and re-exported at the crate root — unlike
/// the label helpers and candidate type above, which stay internal to this
/// crate.
///
/// No production caller in this PR: the exec transport that calls `touch`
/// after every command, and the reaper that reads `idle_for`, both land in a
/// later PR on top of `exec_transport`/`reaper` (out of scope here — see
/// this PR's description). `#[allow(dead_code)]` keeps the lint gate quiet
/// rather than adding a fake caller.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct SandboxActivityRegistry {
    last_activity: Mutex<HashMap<RebornSandboxUserKey, Instant>>,
}

#[allow(dead_code)]
impl SandboxActivityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RebornSandboxUserKey, Instant>> {
        // Recover from poisoning rather than panic: a background reaper
        // must never crash the whole process over a prior panic elsewhere
        // that poisoned this unrelated mutex.
        self.last_activity
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(crate) fn touch(&self, key: &RebornSandboxUserKey) {
        self.lock().insert(key.clone(), Instant::now());
    }

    pub(crate) fn last_activity(&self, key: &RebornSandboxUserKey) -> Option<Instant> {
        self.lock().get(key).copied()
    }

    pub(crate) fn forget(&self, key: &RebornSandboxUserKey) {
        self.lock().remove(key);
    }

    pub(crate) fn idle_for(&self, key: &RebornSandboxUserKey, now: Instant) -> Option<Duration> {
        self.last_activity(key)
            .map(|activity| now.saturating_duration_since(activity))
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
/// [`RebornSandboxUserKey`] the same way [`SandboxActivityRegistry`] is.
/// Kept as a sibling registry (single responsibility) rather than folded
/// into the activity map.
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
    use ironclaw_host_api::{TenantId, UserId};

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

    fn test_key(tenant: &str, user: &str) -> RebornSandboxUserKey {
        let scope = ironclaw_host_api::ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        RebornSandboxUserKey::from_scope(&scope)
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
    fn activity_registry_touch_then_idle_for_reports_elapsed_duration() {
        let registry = SandboxActivityRegistry::new();
        let tenant = TenantId::new("t").unwrap();
        let user = UserId::new("u").unwrap();
        let scope = ironclaw_host_api::ResourceScope {
            tenant_id: tenant,
            user_id: user,
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        let key = RebornSandboxUserKey::from_scope(&scope);

        assert!(registry.last_activity(&key).is_none());
        registry.touch(&key);
        let idle = registry.idle_for(&key, Instant::now() + Duration::from_secs(5));
        assert!(idle.unwrap() >= Duration::from_secs(5));
    }

    #[test]
    fn activity_registry_forget_clears_the_entry() {
        let registry = SandboxActivityRegistry::new();
        let scope = ironclaw_host_api::ResourceScope {
            tenant_id: TenantId::new("t").unwrap(),
            user_id: UserId::new("u").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        let key = RebornSandboxUserKey::from_scope(&scope);
        registry.touch(&key);

        registry.forget(&key);

        assert!(registry.last_activity(&key).is_none());
    }

    #[test]
    fn activity_registry_survives_concurrent_touch_read_and_forget() {
        // `SandboxActivityRegistry` is a shared `Mutex` meant to be hit
        // concurrently by exec transport (touch after every command) and the
        // reaper (idle_for/forget) from separate tasks — every prior test
        // here is strictly sequential. This drives real concurrent access
        // from multiple OS threads: the assertion is simply that it
        // completes without panicking/deadlocking (a poisoned or
        // deadlocked mutex would hang or panic this test) and that state
        // from a still-touching thread is observable afterwards.
        use std::sync::Arc;
        use std::thread;

        let registry = Arc::new(SandboxActivityRegistry::new());
        let keys: Vec<RebornSandboxUserKey> = (0..8)
            .map(|index| test_key("t", &format!("user-{index}")))
            .collect();

        let mut handles = Vec::new();
        for key in keys.clone() {
            let registry = Arc::clone(&registry);
            handles.push(thread::spawn(move || {
                for _ in 0..200 {
                    registry.touch(&key);
                    let _ = registry.last_activity(&key);
                    let _ = registry.idle_for(&key, Instant::now());
                }
            }));
        }
        // A concurrent forget on a disjoint key set, so touch/read on one
        // key and forget on another race on the same underlying map.
        let forget_registry = Arc::clone(&registry);
        let forget_key = test_key("t", "user-forget-target");
        forget_registry.touch(&forget_key);
        handles.push(thread::spawn(move || {
            for _ in 0..200 {
                forget_registry.touch(&forget_key);
                forget_registry.forget(&forget_key);
            }
        }));

        for handle in handles {
            handle
                .join()
                .expect("registry access thread must not panic");
        }

        for key in &keys {
            assert!(registry.last_activity(key).is_some());
        }
    }
}
